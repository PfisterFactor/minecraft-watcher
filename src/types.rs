use std::fmt::{Display, Formatter};

#[derive(Copy, Clone, PartialEq)]
pub enum ServerStatus {
    Offline,
    StartingEC2,
    StartingUp,
    Online,
    ShuttingDown,
    Unknown
}

impl ServerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerStatus::Offline => "Offline",
            ServerStatus::StartingEC2 => "StartingEC2",
            ServerStatus::StartingUp => "StartingUp",
            ServerStatus::Online => "Online",
            ServerStatus::ShuttingDown => "ShuttingDown",
            ServerStatus::Unknown => "Unknown"
        }
    }
    pub fn get_motd(&self) -> &'static str {
        match self {
            ServerStatus::Offline => "&4Offline &f&o(join to start server up)",
            ServerStatus::StartingEC2 => "&6Starting EC2 instance...",
            ServerStatus::StartingUp => "&6Starting minecraft server...",
            // Don't think these two will be used since the server will take over MOTD
            ServerStatus::Online => "&2Online",
            ServerStatus::ShuttingDown => "&cShutting down...",
            ServerStatus::Unknown => "Unknown"
        }
    }
}

impl Display for ServerStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let formatted = "ServerStatus::".to_string() + self.as_str();
        f.write_str(&formatted)
    }
}
