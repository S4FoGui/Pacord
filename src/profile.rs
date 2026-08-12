use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MouseProfile {
    pub sensitivity: f32,
    pub acceleration_curve: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ControllerProfile {
    pub deadzone: f32,
    pub invert_y: bool,
    pub button_map: HashMap<String, u16>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct UserProfile {
    pub nickname: String,
    pub mouse: MouseProfile,
    pub controller: ControllerProfile,
    pub key_remap: HashMap<String, String>,
}

impl Default for UserProfile {
    fn default() -> Self {
        let mut button_map = HashMap::new();
        button_map.insert("A".to_string(), 0x130);
        button_map.insert("B".to_string(), 0x131);
        button_map.insert("X".to_string(), 0x133);
        button_map.insert("Y".to_string(), 0x134);

        Self {
            nickname: "PACORD_User".to_string(),
            mouse: MouseProfile {
                sensitivity: 1.0,
                acceleration_curve: 1.0,
            },
            controller: ControllerProfile {
                deadzone: 0.15,
                invert_y: false,
                button_map,
            },
            key_remap: HashMap::new(),
        }
    }
}

impl UserProfile {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        if !path.as_ref().exists() {
            let default_profile = Self::default();
            default_profile.save_to_file(&path)?;
            return Ok(default_profile);
        }
        let content = fs::read_to_string(path)?;
        let profile: UserProfile = toml::from_str(&content)?;
        Ok(profile)
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::UserProfile;
    use std::fs;

    #[test]
    fn default_profile_round_trips_as_toml() {
        let path =
            std::env::temp_dir().join(format!("pacord-profile-test-{}.toml", std::process::id()));
        let original = UserProfile::default();
        original.save_to_file(&path).expect("profile should save");
        let loaded = UserProfile::load_from_file(&path).expect("profile should load");

        assert_eq!(loaded.nickname, original.nickname);
        assert_eq!(loaded.mouse.sensitivity, original.mouse.sensitivity);
        assert_eq!(loaded.controller.deadzone, original.controller.deadzone);
        assert_eq!(loaded.controller.invert_y, original.controller.invert_y);
        assert_eq!(loaded.key_remap, original.key_remap);

        fs::remove_file(path).expect("temporary profile should be removable");
    }
}
