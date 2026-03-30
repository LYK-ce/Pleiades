//Presented by KeJi
//Date : 2026-03-30

#![allow(non_snake_case, non_camel_case_types)]

use std::path::Path;
use pleiades::{Network_Config, Runtime_Config, Read_Config};

/// 打印 Network 段配置信息
fn Print_Network_Config(network_config: &Network_Config) {
    println!("========== Network Configuration ==========");
    println!(
        "  LAN Enabled          : {}",
        network_config.LAN.unwrap_or(false)
    );
    println!(
        "  WAN Enabled          : {}",
        network_config.WAN.unwrap_or(false)
    );
    println!(
        "  Transport Protocol   : {}",
        network_config
            .Transport_Protocol
            .as_deref()
            .unwrap_or("N/A")
    );
    println!("============================================");
}

/// 打印 Runtime 段配置信息
fn Print_Runtime_Config(runtime_config: &Runtime_Config) {
    println!("========== Runtime Configuration ===========");
    println!(
        "  Device               : {}",
        runtime_config.device.as_deref().unwrap_or("N/A")
    );
    println!(
        "  Model Path           : {}",
        runtime_config.model_path.as_deref().unwrap_or("N/A")
    );
    println!(
        "  Max Token            : {}",
        runtime_config.max_token.map_or("N/A".to_string(), |v| v.to_string())
    );
    println!(
        "  Temperature          : {}",
        runtime_config.temperature.map_or("N/A".to_string(), |v| v.to_string())
    );
    println!(
        "  Seed                 : {}",
        runtime_config.seed.map_or("N/A".to_string(), |v| v.to_string())
    );
    println!("============================================");
}

fn main() {
    let config_path = Path::new("Src/Config/config.toml");

    println!("Reading config from: {}", config_path.display());

    match Read_Config(config_path) {
        Ok(config) => {
            match config.Network {
                Some(ref network_config) => {
                    Print_Network_Config(network_config);
                }
                None => {
                    println!("[Warning] No [Network] section found in config.toml");
                }
            }

            match config.Runtime {
                Some(ref runtime_config) => {
                    Print_Runtime_Config(runtime_config);
                }
                None => {
                    println!("[Warning] No [Runtime] section found in config.toml");
                }
            }
        }
        Err(e) => {
            eprintln!("[Error] Failed to read config file: {}", e);
            std::process::exit(1);
        }
    }
}
