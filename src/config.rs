use std::env;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub view: Option<String>,
    pub charset: Option<String>,
    pub fps: Option<u32>,
    pub speed: Option<f32>,
    pub tilt: Option<f32>,
    pub azimuth: Option<f32>,
    pub roll: Option<f32>,
    pub distance: Option<f32>,
    pub zoom: Option<f32>,
    pub color: Option<String>,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.fps.is_some_and(|fps| !(5..=120).contains(&fps)) {
            return Err(format!(
                "fps in {} must be from 5 to 120",
                config_path().display()
            ));
        }
        for (name, value, min, max) in [
            ("speed", self.speed, 0.05, 8.0),
            ("tilt", self.tilt, -85.0, 85.0),
            ("azimuth", self.azimuth, -360.0, 360.0),
            ("roll", self.roll, -180.0, 180.0),
            ("distance", self.distance, 3.6, 60.0),
            ("zoom", self.zoom, 0.5, 3.0),
        ] {
            if let Some(value) = value
                && (!value.is_finite() || !(min..=max).contains(&value))
            {
                return Err(format!(
                    "{name} in {} must be a finite number from {min} to {max}",
                    config_path().display()
                ));
            }
        }
        Ok(())
    }
}

pub fn load() -> Result<Config, String> {
    let path = config_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Config::default());
        }
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let config: Config = toml::from_str(&text)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    config.validate()?;
    Ok(config)
}

pub fn config_path() -> PathBuf {
    config_home()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("saturn-art")
        .join("config.toml")
}

fn config_home() -> Option<PathBuf> {
    if let Some(path) = env::var_os("XDG_CONFIG_HOME")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    #[cfg(windows)]
    if let Some(path) = env::var_os("APPDATA")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_defaults() {
        let config: Config = toml::from_str(
            "view = \"classic\"\ncharset = \"dense\"\nfps = 30\nspeed = 1.5\ntilt = 12.0\ncolor = \"never\"",
        )
        .unwrap();
        config.validate().unwrap();
        assert_eq!(config.charset.as_deref(), Some("dense"));
        assert_eq!(config.fps, Some(30));
    }

    #[test]
    fn rejects_invalid_and_unknown_settings() {
        for source in [
            "fps = 4",
            "fps = 121",
            "speed = nan",
            "tilt = 90.0",
            "zoom = 0.4",
        ] {
            let config: Config = toml::from_str(source).unwrap();
            assert!(config.validate().is_err(), "{source}");
        }
        assert!(toml::from_str::<Config>("vieuw = \"classic\"").is_err());
    }

    #[test]
    fn locates_the_standard_config_path() {
        assert!(config_path().ends_with("saturn-art/config.toml"));
    }
}
