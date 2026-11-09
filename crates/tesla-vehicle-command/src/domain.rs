// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

/// Vehicle subsystem that owns a session.
///
/// VCSEC and Infotainment have different keys, clocks, and counters. A handshake with one does
/// not authorize commands to the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Domain {
    /// Body controller. Lock, unlock, and other remote-keyless actions.
    VehicleSecurity,
    /// Infotainment. Climate, charging, lights, and horn.
    Infotainment,
}

impl Domain {
    pub(crate) fn code(self) -> u8 {
        match self {
            Self::VehicleSecurity => 2,
            Self::Infotainment => 3,
        }
    }

    pub(crate) fn as_i32(self) -> i32 {
        i32::from(self.code())
    }

    /// Nesting depth of a command protobuf, used by detached encryption.
    pub(crate) fn detached_depth(self) -> usize {
        match self {
            Self::Infotainment => 2,
            Self::VehicleSecurity => 1,
        }
    }

    pub(crate) fn from_code(code: i32) -> Option<Self> {
        match code {
            2 => Some(Self::VehicleSecurity),
            3 => Some(Self::Infotainment),
            _ => None,
        }
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::VehicleSecurity => "vehicle security",
            Self::Infotainment => "infotainment",
        })
    }
}
