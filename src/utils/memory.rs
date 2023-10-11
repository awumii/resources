use std::{process::Command, sync::OnceLock};

use anyhow::{bail, Context, Result};
use nparse::KVStrToJson;
use regex::Regex;
use serde_json::Value;

use super::{FLATPAK_APP_PATH, FLATPAK_SPAWN, IS_FLATPAK};

static RE_SPEED: OnceLock<Regex> = OnceLock::new();
static RE_FORMFACTOR: OnceLock<Regex> = OnceLock::new();
static RE_TYPE: OnceLock<Regex> = OnceLock::new();
static RE_TYPE_DETAIL: OnceLock<Regex> = OnceLock::new();

fn proc_meminfo() -> Result<Value, anyhow::Error> {
    std::fs::read_to_string("/proc/meminfo")
        .with_context(|| "unable to read /proc/meminfo")?
        .kv_str_to_json()
        .map_err(anyhow::Error::msg)
}

pub fn get_total_memory() -> Option<usize> {
    proc_meminfo().ok()?["MemTotal"]
        .as_str()
        .and_then(|x| x.split(' ').collect::<Vec<&str>>()[0].parse::<usize>().ok())
        .map(|y| y * 1000)
}

pub fn get_available_memory() -> Option<usize> {
    proc_meminfo().ok()?["MemAvailable"]
        .as_str()
        .and_then(|x| x.split(' ').collect::<Vec<&str>>()[0].parse::<usize>().ok())
        .map(|y| y * 1000)
}

pub fn get_free_memory() -> Option<usize> {
    proc_meminfo().ok()?["MemFree"]
        .as_str()
        .and_then(|x| x.split(' ').collect::<Vec<&str>>()[0].parse::<usize>().ok())
        .map(|y| y * 1000)
}

pub fn get_total_swap() -> Option<usize> {
    proc_meminfo().ok()?["SwapTotal"]
        .as_str()
        .and_then(|x| x.split(' ').collect::<Vec<&str>>()[0].parse::<usize>().ok())
        .map(|y| y * 1000)
}

pub fn get_free_swap() -> Option<usize> {
    proc_meminfo().ok()?["SwapFree"]
        .as_str()
        .and_then(|x| x.split(' ').collect::<Vec<&str>>()[0].parse::<usize>().ok())
        .map(|y| y * 1000)
}

#[derive(Debug, Clone, Default)]
pub struct MemoryDevice {
    pub speed: Option<u32>,
    pub form_factor: String,
    pub r#type: String,
    pub type_detail: String,
    pub installed: bool,
}

fn parse_dmidecode(dmi: &str) -> Vec<MemoryDevice> {
    let mut devices = Vec::new();

    let device_strings = dmi.split("\n\n");

    let re_speed = RE_SPEED.get_or_init(|| Regex::new(r"Speed: (\d+) MT/s").unwrap());
    let re_form_factor = RE_FORMFACTOR.get_or_init(|| Regex::new(r"Form Factor: (.+)").unwrap());
    let re_type = RE_TYPE.get_or_init(|| Regex::new(r"Type: (.+)").unwrap());
    let re_type_detail = RE_TYPE_DETAIL.get_or_init(|| Regex::new(r"Type Detail: (.+)").unwrap());

    for device_string in device_strings {
        if device_string.is_empty() {
            continue;
        }
        let memory_device = MemoryDevice {
            speed: re_speed
                .captures(device_string)
                .map(|x| x[1].parse().unwrap()),
            form_factor: re_form_factor
                .captures(device_string)
                .map_or_else(|| "N/A".to_string(), |x| x[1].to_string()),
            r#type: re_type
                .captures(device_string)
                .map_or_else(|| "N/A".to_string(), |x| x[1].to_string()),
            type_detail: re_type_detail
                .captures(device_string)
                .map_or_else(|| "N/A".to_string(), |x| x[1].to_string()),
            installed: re_speed
                .captures(device_string)
                .map(|x| x[1].to_string())
                .is_some(),
        };

        devices.push(memory_device);
    }

    devices
}

pub fn get_memory_devices() -> Result<Vec<MemoryDevice>> {
    let output = Command::new("dmidecode")
        .args(["-t", "17", "-q"])
        .output()?;
    if output.status.code().unwrap_or(1) == 1 {
        bail!("no permission")
    }
    Ok(parse_dmidecode(String::from_utf8(output.stdout)?.as_str()))
}

pub fn pkexec_get_memory_devices() -> Result<Vec<MemoryDevice>> {
    let output = if *IS_FLATPAK {
        Command::new(FLATPAK_SPAWN)
            .args([
                "--host",
                "/usr/bin/pkexec",
                "--disable-internal-agent",
                &format!("{}/bin/dmidecode", FLATPAK_APP_PATH.as_str()),
                "-t",
                "17",
                "-q",
            ])
            .output()?
    } else {
        Command::new("pkexec")
            .args(["--disable-internal-agent", "dmidecode", "-t", "17", "-q"])
            .output()?
    };
    Ok(parse_dmidecode(String::from_utf8(output.stdout)?.as_str()))
}
