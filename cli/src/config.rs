use crate::{
    util::{config_path, data_path, screenshot_path, unix_timestamp},
    ITEMS_CACHE_EXPIRY_S,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::SystemTime;
use text_io;
use wfm_rs::response::ShortItem;

type JwtToken = String;

#[derive(Serialize, Deserialize)]
pub struct Config {
    jwt_token: JwtToken,
    items_timestamp: u64,
    pub items: Vec<wfm_rs::response::ShortItem>,
}

impl Config {
    pub fn user(&self) -> wfm_rs::User {
        wfm_rs::User::_from_jwt_token(&self.jwt_token)
    }
}

#[allow(unused_must_use)]
pub async fn run() -> Result<Config> {
    let data_path = data_path()?;
    let data_path_screenshot = screenshot_path()?;
    let data_path_config = config_path()?;

    let mut config = {
        if let Ok(mut file) = File::open(&data_path_config) {
            let mut strbuf = String::new();
            file.read_to_string(&mut strbuf)?;
            let mut cfg = serde_json::from_str::<Config>(&strbuf)?;

            if (unix_timestamp()? - cfg.items_timestamp) > ITEMS_CACHE_EXPIRY_S {
                print!("Refreshing items...   ");
                let mut items = wfm_rs::User::_from_jwt_token(&cfg.jwt_token)
                    .get_items()
                    .await?;
                fix_items(&mut items);
                cfg.items = items;
                cfg.items_timestamp = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)?
                    .as_secs();
                write_config_to_file(&data_path_config, &cfg)?;
                println!("success!");
            }

            cfg
        } else {
            fs::create_dir(&data_path);
            fs::create_dir(&data_path_screenshot);
            File::create(&data_path_config);

            let token = JwtToken::default();

            print!("Building config...   ");
            let cfg = Config {
                items: wfm_rs::User::_from_jwt_token(&token).get_items().await?,
                items_timestamp: unix_timestamp()?,
                jwt_token: token,
            };
            println!("success!");

            write_config_to_file(&data_path_config, &cfg)?;

            cfg
        }
    };

    config.items.push(ShortItem {
        url_name: "".to_string(),
        thumb: "".to_string(),
        id: "".to_string(),
        item_name: "Forma Blueprint".to_string(),
    });

    Ok(config)
}

// guide user through login process
async fn login_process() -> Result<JwtToken> {
    println!("You need to log in with your warframe.market account!");
    println!("This program does not store your e-mail and/or password, they are both only used once, to log into the warframe.market API.");
    println!("The only thing related to your account this program stores is the token received from the API.");

    let email = prompt("E-mail:");
    let password = prompt("Password:");
    let platform = prompt("Platform (pc, xbox or ps4):");

    println!("\n");

    print!("Fetching token from API...   ");
    let user = wfm_rs::User::login(&email, &password, &platform, "en").await?;
    println!("success!");

    Ok(user._jwt_token())
}

fn prompt(text: &str) -> String {
    println!("\n{}", text);
    text_io::read!("{}\n")
}

fn write_config_to_file(path: &PathBuf, config: &Config) -> Result<()> {
    let mut file = fs::OpenOptions::new().write(true).open(path)?;
    let config_str = serde_json::to_string(config)?;
    let bytes = config_str.as_bytes();
    let written = file.write(&bytes)?;
    if written < bytes.len() {
        anyhow::bail!("Not all bytes written!");
    }
    Ok(())
}

fn fix_items(items: &mut Vec<ShortItem>) {
    for i in items.iter_mut() {
        if i.item_name.contains("Neuroptics")
            | i.item_name.contains("Systems")
            | i.item_name.contains("Chassis")
        {
            i.item_name.push_str(" blueprint")
        }
    }
}
