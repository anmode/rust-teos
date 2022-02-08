//! Logic related to appointments handled by the tower.

use hex;
use std::array::TryFromSliceError;
use std::convert::TryInto;
use std::fmt;

use bitcoin::hashes::{ripemd160, Hash};

use crate::protos as msgs;
use teos_common::appointment::{Appointment, Locator};
use teos_common::UserId;

/// Unique identifier used to identify appointments.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct UUID([u8; 20]);

impl UUID {
    /// Creates a new [UUID].
    ///
    /// The [UUID]s are created as the `RIPEMD160(locator || user_id)`. This makes it easy to retrieve an [ExtendedAppointment] from the tower
    /// when a user requests it without having to perform lookups based on the [Locator], and match what [UUID] belongs to what user (if any).
    /// Therefore, it provides a hard-to-forge id while reducing the tower lookups and the required data to be stored (no reverse maps).
    pub fn new(locator: Locator, user_id: UserId) -> Self {
        let mut uuid_data = locator.serialize();
        uuid_data.extend(&user_id.0.serialize());
        UUID(ripemd160::Hash::hash(&uuid_data).into_inner())
    }

    /// Serializes the [UUID] returning its byte representation.
    pub fn serialize(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Builds a [UUID] from its byte representation.
    pub fn deserialize(data: &[u8]) -> Result<Self, TryFromSliceError> {
        data.try_into().map(|x| Self(x))
    }
}

impl std::fmt::Display for UUID {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

/// An extended version of the appointment hold by the tower.
///
/// The [Appointment] is extended in terms of data, that is, it provides further information only relevant to the tower.
/// Notice [ExtendedAppointment]s are not kept in memory but persisted on disk. The [Watcher](crate::watcher::Watcher)
/// keeps [AppointmentSummary] instead.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct ExtendedAppointment {
    /// The underlying appointment extended by [ExtendedAppointment].
    pub inner: Appointment,
    /// The user this [Appointment] belongs to.
    pub user_id: UserId,
    /// The signature provided by the user when handing the [Appointment].
    pub user_signature: String,
    /// The block where the [Appointment] is started to be watched at by the [Watcher](crate::watcher::Watcher).
    pub start_block: u32,
}

/// A summary of an appointment.
///
/// Contains the minimal amount of data the [Watcher](crate::watcher::Watcher) needs to keep in memory in order to
/// watch for breaches.
#[derive(Debug, Eq, PartialEq, Clone)]
pub struct AppointmentSummary {
    /// The [Appointment] locator.
    pub locator: Locator,
    /// The user this [Appointment] belongs to.
    pub user_id: UserId,
}

impl ExtendedAppointment {
    /// Create a new [ExtendedAppointment].
    pub fn new(
        inner: Appointment,
        user_id: UserId,
        user_signature: String,
        start_block: u32,
    ) -> Self {
        ExtendedAppointment {
            inner,
            user_id,
            user_signature,
            start_block,
        }
    }

    /// Gets the underlying appointment's locator.
    pub fn locator(&self) -> Locator {
        self.inner.locator
    }

    /// Gets the underlying appointment's encrypted data blob
    pub fn encrypted_blob(&self) -> &Vec<u8> {
        &self.inner.encrypted_blob
    }

    /// Gets the underlying appointment's `to_self_delay`
    pub fn to_self_delay(&self) -> u32 {
        self.inner.to_self_delay
    }

    /// Computes the summary of the [ExtendedAppointment].
    pub fn get_summary(&self) -> AppointmentSummary {
        AppointmentSummary {
            locator: self.locator(),
            user_id: self.user_id,
        }
    }
}

impl Into<msgs::Appointment> for Appointment {
    fn into(self) -> msgs::Appointment {
        msgs::Appointment {
            locator: self.locator.serialize(),
            encrypted_blob: self.encrypted_blob.clone(),
            to_self_delay: self.to_self_delay,
        }
    }
}

/// Computes the number of slots an appointment takes from a user subscription.
///
/// This is based on the [encrypted_blob](Appointment::encrypted_blob) size and the slot size that was defined by the [Gatekeeper](crate::gatekeeper::Gatekeeper).
pub fn compute_appointment_slots(blob_size: usize, blob_max_size: usize) -> u32 {
    (blob_size as f32 / blob_max_size as f32).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::test_utils::get_random_user_id;
    use teos_common::appointment::Appointment;
    use teos_common::cryptography::get_random_bytes;

    #[test]
    fn test_get_summary() {
        let locator = Locator::deserialize(&get_random_bytes(16)).unwrap();
        let user_id = get_random_user_id();
        let signature = String::new();

        let a = Appointment::new(locator, get_random_bytes(32), 42);
        let e = ExtendedAppointment::new(a, user_id, signature, 21);

        let s = e.get_summary();

        assert_eq!(e.locator(), s.locator);
        assert_eq!(e.user_id, s.user_id);
    }
}
