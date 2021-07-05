use anyhow::Result;
use colored::*;
use device_query::{DeviceQuery, DeviceState, Keycode};
use ocr::OCREngine;
use screenshot_rs;
use std::fs::File;
use std::io::Write;
use std::{fs, thread, time::Duration};
use tokio;
use util::{clear_terminal, screenshot_path, unix_timestamp};
use wfm_rs::response::ShortItem;
use wfm_rs::User;

mod config;
mod ocr;
mod util;

const DATA_PATH_SUFFIX: &str = ".wfm_cli/";
const DATA_TESSDATA_DIR: &str = "tessdata/";
const DATA_SCREENSHOT_DIR: &str = "screenshots/";
const DATA_CONFIG_FILE: &str = "config.wfm.json";
const ITEMS_CACHE_EXPIRY_S: u64 = 24 * 60 * 60;
const RESULT_COLORS: [Color; 4] = [
    Color::TrueColor { r: 0, g: 255, b: 8 },
    Color::TrueColor {
        r: 255,
        g: 174,
        b: 9,
    },
    Color::TrueColor {
        r: 255,
        g: 99,
        b: 9,
    },
    Color::TrueColor {
        r: 255,
        g: 12,
        b: 9,
    },
];

// TODO:
// - silence tesseract dpi complaining
// - release wfm_rs
// - release cli

#[cfg(target_os = "windows")]
std::compile_error!("Windows is not supported!");

#[tokio::main]
async fn main() {
    let config = config::run().await.unwrap();
    let user = config.user();
    let device = DeviceState::new();
    let engine = OCREngine::new(config.items);
    println!("You may now press 'F6' whenever you get to the relic reward screen");

    {
        let mut data_path = home::home_dir().unwrap();
        data_path.push(DATA_PATH_SUFFIX);
        data_path.push(DATA_TESSDATA_DIR);

        let user_words = include_str!("../tessdata/eng.user-words");
        let traineddata = include_bytes!("../tessdata/eng.traineddata");

        let _ = fs::create_dir_all(data_path.clone());

        let mut user_words_file = File::create(data_path.join("eng.user-words")).unwrap();
        write!(user_words_file, "{}", user_words).unwrap();

        let mut traineddata_file = File::create(data_path.join("eng.traineddata")).unwrap();
        traineddata_file.write(traineddata).unwrap();
    }

    loop {
        let keys: Vec<Keycode> = device.get_keys();
        if keys.contains(&Keycode::F6) {
            println!("Scanning...");
            let mut screenshot_path = screenshot_path().unwrap();
            screenshot_path.push(format!("{}.png", unix_timestamp().unwrap()));
            let screenshot_path_str = screenshot_path.to_string_lossy().to_string();
            screenshot_rs::screenshot_window(screenshot_path_str.clone());
            let items = engine.ocr(&screenshot_path_str).unwrap();
            fs::remove_file(screenshot_path).unwrap();

            let mut all_item_stats = Vec::new();

            let mut best_idx = 0;
            let mut best_price = 0.0;

            for (i, item) in items {
                let item_stats = get_item_info(&item, &user).await.unwrap();
                if !item_stats.avg_price.is_nan() {
                    if item_stats.avg_price > best_price {
                        best_price = item_stats.avg_price;
                        best_idx = i;
                    }
                    all_item_stats.push(item_stats);
                }
            }

            all_item_stats.sort_by(|a, b| a.avg_price.partial_cmp(&b.avg_price).unwrap());
            let all_item_stats: Vec<&ItemStats> = all_item_stats.iter().rev().collect();

            clear_terminal();

            for (idx, item) in all_item_stats.iter().enumerate() {
                let msg = format!(
                    "{} | {:.1} platinum average | {:.0} sold in the last 48 hours",
                    item.item.item_name, item.avg_price, item.volume
                );
                println!("{}", msg.color(RESULT_COLORS[idx]));
            }
            let _ = beep(best_idx + 1).await;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Clone)]
struct ItemStats {
    volume: f32,
    avg_price: f32,
    item: ShortItem,
}

async fn get_item_info(item: &ShortItem, user: &User) -> Result<ItemStats> {
    let statistics = user.get_item_market_statistics(item).await?;

    let last_stats = &statistics.statistics_closed._48_hours;
    let avg_price: f32 =
        last_stats.iter().map(|x| x.avg_price).sum::<f32>() / last_stats.len() as f32;
    let volume: f32 = last_stats.iter().map(|x| x.volume).sum();

    Ok(ItemStats {
        volume,
        avg_price,
        item: item.clone(),
    })
}

async fn beep(times: usize) -> Result<()> {
    use rodio::{
        source::{SineWave, Source},
        OutputStream, Sink,
    };

    static BEEP_FREQUENCY: u32 = 587;
    static BEEP_DURATION: f32 = 0.10;
    static BEEP_SEPARATION: f32 = 0.05;
    static LENGTH_PER_BEEP: f32 = BEEP_DURATION + BEEP_SEPARATION;

    let total_length = LENGTH_PER_BEEP * times as f32;

    let (_stream, stream_handle) = OutputStream::try_default()?;
    let sink = Sink::try_new(&stream_handle)?;

    let sine = SineWave::new(BEEP_FREQUENCY)
        .take_duration(Duration::from_secs_f32(BEEP_DURATION))
        .amplify(0.1);
    let mixed = sine.mix(SineWave::new(0).take_duration(Duration::from_secs_f32(LENGTH_PER_BEEP)));
    let repeated = mixed
        .repeat_infinite()
        .take_duration(Duration::from_secs_f32(total_length));

    sink.append(repeated);
    tokio::time::sleep(Duration::from_secs_f32(total_length + BEEP_DURATION * 2.0)).await;

    Ok(())
}
