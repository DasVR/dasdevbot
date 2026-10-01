//! Who owns each class of data. Device chats never replicate to the server.
//! The device-to-server payload is job metadata only.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataOwner {
    Device,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnedData {
    pub class: &'static str,
    pub owner: DataOwner,
    /// Included in the default device → server replica.
    pub replicate_to_server: bool,
}

pub const OWNERSHIP: &[OwnedData] = &[
    OwnedData {
        class: "chat",
        owner: DataOwner::Device,
        replicate_to_server: false,
    },
    OwnedData {
        class: "chat_message",
        owner: DataOwner::Device,
        replicate_to_server: false,
    },
    OwnedData {
        class: "secret",
        owner: DataOwner::Device,
        replicate_to_server: false,
    },
    OwnedData {
        class: "approval_decision",
        owner: DataOwner::Device,
        replicate_to_server: false,
    },
    OwnedData {
        class: "job_metadata",
        owner: DataOwner::Server,
        replicate_to_server: true,
    },
    OwnedData {
        class: "provider_cap_ledger",
        owner: DataOwner::Server,
        replicate_to_server: false,
    },
];

pub fn device_to_server_classes() -> Vec<&'static str> {
    OWNERSHIP
        .iter()
        .filter(|row| row.replicate_to_server)
        .map(|row| row.class)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chats_stay_on_the_device_and_the_server_gets_job_metadata() {
        let chat = OWNERSHIP.iter().find(|row| row.class == "chat").unwrap();
        assert_eq!(chat.owner, DataOwner::Device);
        assert!(!chat.replicate_to_server);
        let message = OWNERSHIP
            .iter()
            .find(|row| row.class == "chat_message")
            .unwrap();
        assert!(!message.replicate_to_server);
        assert_eq!(device_to_server_classes(), vec!["job_metadata"]);
    }
}
