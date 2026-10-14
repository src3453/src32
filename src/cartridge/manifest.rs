use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub id: String,
    pub title: String,
    pub system: System,
    pub entrypoint: Entrypoint,
    #[serde(default)]
    pub disks: Vec<Disk>,
    #[serde(default)]
    pub plugins: Vec<Plugin>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct System {
    pub architecture: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entrypoint {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Disk {
    pub slot: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub mode: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plugin {
    pub slot: u8,
    pub path: String,
}
