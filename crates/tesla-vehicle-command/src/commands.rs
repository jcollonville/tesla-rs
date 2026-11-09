// SPDX-License-Identifier: MIT OR Apache-2.0

use prost::Message;

use crate::CommandError;
use crate::domain::Domain;
use crate::proto::car_server::action::ActionMsg;
use crate::proto::car_server::charging_start_stop_action::ChargingAction;
use crate::proto::car_server::hvac_temperature_adjustment_action::Temperature;
use crate::proto::car_server::hvac_temperature_adjustment_action::temperature::Type as TempType;
use crate::proto::car_server::result_reason::Reason;
use crate::proto::car_server::vehicle_action::VehicleActionMsg;
use crate::proto::car_server::{
    Action, ActionStatus, ChargingSetLimitAction, ChargingStartStopAction, HvacAutoAction,
    HvacTemperatureAdjustmentAction, OperationStatusE as CarStatus, Response,
    SetChargingAmpsAction, VehicleAction, VehicleControlFlashLightsAction,
    VehicleControlHonkHornAction, Void,
};
use crate::proto::errors::GenericErrorE;
use crate::proto::vcsec::command_status::SubMessage as CommandSub;
use crate::proto::vcsec::from_vcsec_message::SubMessage as VcsecSub;
use crate::proto::vcsec::unsigned_message::SubMessage as UnsignedSub;
use crate::proto::vcsec::{
    FromVcsecMessage, OperationStatusE as VcsecStatus, RkeActionE, UnsignedMessage,
};

pub(crate) fn lock() -> Vec<u8> {
    rke(RkeActionE::RkeActionLock)
}

pub(crate) fn unlock() -> Vec<u8> {
    rke(RkeActionE::RkeActionUnlock)
}

pub(crate) fn climate(on: bool) -> Vec<u8> {
    car(VehicleActionMsg::HvacAutoAction(HvacAutoAction {
        power_on: on,
        manual_override: false,
    }))
}

pub(crate) fn temperatures(driver: f32, passenger: f32) -> Result<Vec<u8>, CommandError> {
    check_temp(driver)?;
    check_temp(passenger)?;
    Ok(car(VehicleActionMsg::HvacTemperatureAdjustmentAction(
        HvacTemperatureAdjustmentAction {
            driver_temp_celsius: driver,
            passenger_temp_celsius: passenger,
            level: Some(Temperature {
                r#type: Some(TempType::TempMax(Void {})),
            }),
            ..HvacTemperatureAdjustmentAction::default()
        },
    )))
}

pub(crate) fn charge_start() -> Vec<u8> {
    charge(ChargingAction::Start(Void {}))
}

pub(crate) fn charge_stop() -> Vec<u8> {
    charge(ChargingAction::Stop(Void {}))
}

pub(crate) fn charge_limit(percent: i32) -> Result<Vec<u8>, CommandError> {
    if !(0..=100).contains(&percent) {
        return Err(CommandError::InvalidChargeLimit);
    }
    Ok(car(VehicleActionMsg::ChargingSetLimitAction(
        ChargingSetLimitAction { percent },
    )))
}

pub(crate) fn charging_amps(amps: i32) -> Result<Vec<u8>, CommandError> {
    if !(1..=1000).contains(&amps) {
        return Err(CommandError::InvalidAmps);
    }
    Ok(car(VehicleActionMsg::SetChargingAmpsAction(
        SetChargingAmpsAction {
            charging_amps: amps,
        },
    )))
}

pub(crate) fn honk() -> Vec<u8> {
    car(VehicleActionMsg::VehicleControlHonkHornAction(
        VehicleControlHonkHornAction {},
    ))
}

pub(crate) fn flash() -> Vec<u8> {
    car(VehicleActionMsg::VehicleControlFlashLightsAction(
        VehicleControlFlashLightsAction {},
    ))
}

pub(crate) fn interpret(domain: Domain, payload: &[u8]) -> Result<(), CommandError> {
    if payload.is_empty() {
        return Ok(());
    }
    match domain {
        Domain::VehicleSecurity => interpret_vcsec(payload),
        Domain::Infotainment => interpret_car(payload),
    }
}

fn rke(action: RkeActionE) -> Vec<u8> {
    UnsignedMessage {
        sub_message: Some(UnsignedSub::RkeAction(i32::from(action))),
    }
    .encode_to_vec()
}

fn charge(action: ChargingAction) -> Vec<u8> {
    car(VehicleActionMsg::ChargingStartStopAction(
        ChargingStartStopAction {
            charging_action: Some(action),
        },
    ))
}

fn car(action: VehicleActionMsg) -> Vec<u8> {
    Action {
        action_msg: Some(ActionMsg::VehicleAction(VehicleAction {
            vehicle_action_msg: Some(action),
        })),
    }
    .encode_to_vec()
}

fn check_temp(value: f32) -> Result<(), CommandError> {
    if value.is_finite() && (0.0..=50.0).contains(&value) {
        Ok(())
    } else {
        Err(CommandError::InvalidTemperature)
    }
}

fn interpret_vcsec(payload: &[u8]) -> Result<(), CommandError> {
    let message = FromVcsecMessage::decode(payload).map_err(|_| CommandError::BadResponse {
        may_have_succeeded: true,
    })?;
    match message.sub_message {
        Some(VcsecSub::NominalError(error)) => Err(CommandError::nominal(
            Domain::VehicleSecurity,
            generic_error(error.generic_error),
        )),
        Some(VcsecSub::CommandStatus(status)) => match status.operation_status {
            code if code == i32::from(VcsecStatus::OperationstatusWait) => Err(CommandError::Busy),
            code if code == i32::from(VcsecStatus::OperationstatusError) => {
                if let Some(CommandSub::WhitelistOperationStatus(whitelist)) = &status.sub_message
                    && whitelist.whitelist_operation_information != 0
                {
                    return Err(CommandError::Whitelist(
                        whitelist.whitelist_operation_information,
                    ));
                }
                Err(CommandError::nominal(
                    Domain::VehicleSecurity,
                    "command failed",
                ))
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

fn interpret_car(payload: &[u8]) -> Result<(), CommandError> {
    let response = Response::decode(payload).map_err(|_| CommandError::BadResponse {
        may_have_succeeded: true,
    })?;
    let Some(status) = response.action_status else {
        return Ok(());
    };
    if status.result != i32::from(CarStatus::OperationstatusError) {
        return Ok(());
    }
    Err(CommandError::nominal(
        Domain::Infotainment,
        reason_text(&status),
    ))
}

fn reason_text(status: &ActionStatus) -> String {
    match status
        .result_reason
        .as_ref()
        .and_then(|reason| reason.reason.as_ref())
    {
        Some(Reason::PlainText(text)) if !text.is_empty() => text.clone(),
        _ => "unspecified error".to_owned(),
    }
}

fn generic_error(code: i32) -> &'static str {
    match GenericErrorE::try_from(code) {
        Ok(GenericErrorE::GenericerrorClosuresOpen) => "closures open",
        Ok(GenericErrorE::GenericerrorAlreadyOn) => "already on",
        Ok(GenericErrorE::GenericerrorDisabledForUserCommand) => "disabled for user command",
        Ok(GenericErrorE::GenericerrorVehicleNotInPark) => "vehicle not in park",
        Ok(GenericErrorE::GenericerrorUnauthorized) => "unauthorized",
        Ok(GenericErrorE::GenericerrorNotAllowedOverTransport) => "not allowed over this transport",
        Ok(GenericErrorE::GenericerrorUnknown) => "unknown vehicle error",
        _ => "vehicle error",
    }
}
