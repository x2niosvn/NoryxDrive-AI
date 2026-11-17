use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CropBox {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkey_vk: Option<i32>,
    pub hotkey_name: Option<String>,
    pub crop_box: Option<CropBox>,
    pub hotkey_type: Option<String>,
    pub hotkey_code: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey_vk: Some(6),
            hotkey_name: Some("Chuột phụ 2 (Mouse 5)".to_string()),
            crop_box: Some(CropBox {
                left: 456,
                top: 279,
                width: 1385,
                height: 705,
            }),
            hotkey_type: Some("mouse".to_string()),
            hotkey_code: Some("x2".to_string()),
        }
    }
}

impl AppConfig {
    pub fn load<P: AsRef<Path>>(path: P) -> Self {
        if let Ok(data) = fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<AppConfig>(&data) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) {
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, data);
        }
    }
}

