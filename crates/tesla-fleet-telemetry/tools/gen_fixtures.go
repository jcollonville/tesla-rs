// Generate binary fixtures with the reference server.
//
//   cd /tmp && mkdir gen-fixtures && cd gen-fixtures
//   # copy this file to main.go
//   go mod init genfixtures
//   go mod edit -replace github.com/teslamotors/fleet-telemetry=/path/to/fleet-telemetry
//   go get github.com/teslamotors/fleet-telemetry@v0.0.0
//   go mod tidy
//   go run . /path/to/crates/tesla-fleet-telemetry/tests/fixtures
//
// The module must be teslamotors/fleet-telemetry commit
// bb7c8a931e46da3b3eb6d20a67541c8f755f1ee6 or a tree that still exposes
// messages.StreamMessage and protos.Payload.

package main

import (
	"os"
	"path/filepath"
	"time"

	"github.com/teslamotors/fleet-telemetry/messages"
	"github.com/teslamotors/fleet-telemetry/protos"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/timestamppb"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}

func write(dir, name string, b []byte) {
	must(os.WriteFile(filepath.Join(dir, name), b, 0o644))
}

func main() {
	if len(os.Args) != 2 {
		panic("usage: gen-fixtures <dir>")
	}
	dir := os.Args[1]
	created := timestamppb.New(time.Unix(1_700_000_000, 125_000_000))

	payload := &protos.Payload{
		Vin:       "PAYLOADVIN00000001",
		CreatedAt: created,
		IsResend:  true,
		Data: []*protos.Datum{
			{Key: protos.Field_Soc, Value: &protos.Value{Value: &protos.Value_DoubleValue{DoubleValue: 81.5}}},
			{Key: protos.Field_VehicleSpeed, Value: &protos.Value{Value: &protos.Value_FloatValue{FloatValue: 12.25}}},
			{Key: protos.Field_Odometer, Value: &protos.Value{Value: &protos.Value_LongValue{LongValue: 4242}}},
			{Key: protos.Field_ChargeAmps, Value: &protos.Value{Value: &protos.Value_IntValue{IntValue: 32}}},
			{Key: protos.Field_Locked, Value: &protos.Value{Value: &protos.Value_BooleanValue{BooleanValue: true}}},
			{Key: protos.Field_VehicleName, Value: &protos.Value{Value: &protos.Value_StringValue{StringValue: "Roadster"}}},
			{Key: protos.Field_Location, Value: &protos.Value{Value: &protos.Value_StringValue{StringValue: "(37.412374 N, 122.145867 W)"}}},
			{Key: protos.Field_PackVoltage, Value: &protos.Value{Value: &protos.Value_StringValue{StringValue: "1.5e-3"}}},
			{Key: protos.Field_Gear, Value: &protos.Value{Value: &protos.Value_ShiftStateValue{ShiftStateValue: protos.ShiftState_ShiftStateD}}},
			{Key: protos.Field_DetailedChargeState, Value: &protos.Value{Value: &protos.Value_DetailedChargeStateValue{DetailedChargeStateValue: protos.DetailedChargeStateValue_DetailedChargeStateCharging}}},
			{Key: protos.Field_DestinationLocation, Value: &protos.Value{Value: &protos.Value_LocationValue{LocationValue: &protos.LocationValue{Latitude: 48.8566, Longitude: 2.3522}}}},
			{Key: protos.Field_DoorState, Value: &protos.Value{Value: &protos.Value_DoorValue{DoorValue: &protos.Doors{DriverFront: true, TrunkRear: true}}}},
			{Key: protos.Field_ScheduledChargingStartTime, Value: &protos.Value{Value: &protos.Value_TimeValue{TimeValue: &protos.Time{Hour: 22, Minute: 15, Second: 0}}}},
			{Key: protos.Field_TpmsPressureFl, Value: &protos.Value{Value: &protos.Value_Invalid{Invalid: true}}},
			{Key: protos.Field_AutomaticEmergencyBrakingState, Value: &protos.Value{Value: &protos.Value_AutomaticEmergencyBrakingStateValue{AutomaticEmergencyBrakingStateValue: 0}}},
			{Key: protos.Field_GpsState, Value: &protos.Value{Value: &protos.Value_IntValue{IntValue: 7}}},
		},
	}
	payloadBytes, err := proto.Marshal(payload)
	must(err)
	write(dir, "vehicle_data.pb", payloadBytes)

	alerts := &protos.VehicleAlerts{
		Vin:       "PAYLOADVIN00000001",
		CreatedAt: created,
		Alerts: []*protos.VehicleAlert{
			{
				Name:      "VCFRONT_a999",
				Audiences: []protos.Audience{protos.Audience_Customer, protos.Audience_Service},
				StartedAt: timestamppb.New(time.Unix(1_700_000_100, 0)),
				EndedAt:   &timestamppb.Timestamp{Seconds: 1_700_000_200_000},
			},
		},
	}
	alertsBytes, err := proto.Marshal(alerts)
	must(err)
	write(dir, "vehicle_alerts.pb", alertsBytes)

	errorsMsg := &protos.VehicleErrors{
		Vin:       "PAYLOADVIN00000001",
		CreatedAt: created,
		Errors: []*protos.VehicleError{
			{
				Name:      "telemetry_config",
				Body:      "rejected",
				CreatedAt: created,
				Tags:      map[string]string{"reason": "ca", "field": "Location"},
			},
		},
	}
	errorsBytes, err := proto.Marshal(errorsMsg)
	must(err)
	write(dir, "vehicle_errors.pb", errorsBytes)

	connectivity := &protos.VehicleConnectivity{
		Vin:              "PAYLOADVIN00000001",
		ConnectionId:     "socket-1",
		Status:           protos.ConnectivityEvent_CONNECTED,
		CreatedAt:        created,
		NetworkInterface: "wifi",
	}
	connectivityBytes, err := proto.Marshal(connectivity)
	must(err)
	write(dir, "vehicle_connectivity.pb", connectivityBytes)

	stream, err := (&messages.StreamMessage{
		TXID:               []byte("txid-1"),
		MessageTopic:       []byte("V"),
		SenderID:           []byte("vehicle_device.5YJ3E1EA7KF000001"),
		DeviceType:         []byte("vehicle_device"),
		DeviceID:           []byte("5YJ3E1EA7KF000001"),
		Payload:            payloadBytes,
		CreatedAt:          1_700_000_000,
		EnvMessageID:       []byte("mid-1"),
		DeliveredAtEpochMs: 1_700_000_000_123,
	}).ToBytes()
	must(err)
	write(dir, "stream_vehicle_data.bin", stream)

	round, err := messages.StreamMessageFromBytes(stream)
	must(err)
	if string(round.TXID) != "txid-1" || string(round.MessageTopic) != "V" || string(round.DeviceID) != "5YJ3E1EA7KF000001" {
		panic("round trip mismatch")
	}
	if string(round.Payload) != string(payloadBytes) {
		panic("payload mismatch")
	}
	again, err := round.ToBytes()
	must(err)
	if string(again) != string(stream) {
		panic("re-encode mismatch")
	}

	ack, err := (&messages.StreamAckMessage{TXID: []byte("txid-1"), MessageTopic: []byte("V")}).ToBytes()
	must(err)
	write(dir, "stream_ack.bin", ack)
	ackRound, err := messages.StreamAckMessageFromBytes(ack)
	must(err)
	if string(ackRound.TXID) != "txid-1" || ackRound.Topic() != "V" {
		panic("ack round trip")
	}

	errMsg, err := (&messages.StreamMessage{TXID: []byte("txid-1"), Payload: []byte("incorrect message format")}).ToBytes()
	must(err)
	write(dir, "stream_error.bin", errMsg)

	alertsStream, err := (&messages.StreamMessage{
		TXID:         []byte("txid-alerts"),
		MessageTopic: []byte("alerts"),
		SenderID:     []byte("vehicle_device.5YJ3E1EA7KF000001"),
		DeviceType:   []byte("vehicle_device"),
		DeviceID:     []byte("5YJ3E1EA7KF000001"),
		Payload:      alertsBytes,
		CreatedAt:    1_700_000_000,
	}).ToBytes()
	must(err)
	write(dir, "stream_alerts.bin", alertsStream)
}
