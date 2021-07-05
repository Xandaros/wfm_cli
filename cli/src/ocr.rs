use crate::{util::unix_timestamp, DATA_PATH_SUFFIX, DATA_SCREENSHOT_DIR};
use anyhow::Result;
use crossbeam_channel::{unbounded, Receiver, Sender};
use home;
use image::{DynamicImage, GenericImage, GenericImageView, Pixel};
use levenshtein::levenshtein;
use std::sync::{Arc, RwLock};
use std::{fs, thread};
use tesseract;
use wfm_rs::response::ShortItem;

const IMG_MAX_WHITE_DEV: f32 = 45.0;
const ITEM_CROP_SIZE: [u32; 2] = [250, 50];
const ITEM_CROP_COORDS: [[u32; 2]; 4] = [[470, 410], [720, 410], [960, 410], [1200, 410]];

pub struct OCREngine {
    tx: [Sender<DynamicImage>; 4],
    rx: Receiver<(usize, ShortItem)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Hsv {
    h: f64,
    s: f64,
    v: f64,
}

impl Hsv {
    fn new(h: f64, s: f64, v: f64) -> Self {
        Self { h, s, v }
    }
}

impl From<(f64, f64, f64)> for Hsv {
    fn from((h, s, v): (f64, f64, f64)) -> Self {
        Hsv::new(h, s, v)
    }
}

impl OCREngine {
    pub fn new(items: Vec<ShortItem>) -> OCREngine {
        let img_channels: [(Sender<DynamicImage>, Receiver<DynamicImage>); 4] =
            [unbounded(), unbounded(), unbounded(), unbounded()];

        let (ret_channel_tx, ret_channel_rx) = unbounded::<(usize, ShortItem)>();
        let items = Arc::new(RwLock::new(items));

        for i in 0..4 {
            let thread_rx = img_channels[i].1.clone();
            let thread_tx = ret_channel_tx.clone();
            let thread_items = items.clone();
            let _ = thread::spawn(move || {
                let rx = thread_rx;
                let tx = thread_tx;
                let items = thread_items;
                let idx = i;

                let mut data_path = home::home_dir().unwrap();
                data_path.push(DATA_PATH_SUFFIX);

                let mut ts = {
                    let ts = tesseract::Tesseract::new_with_oem(
                        Some(""),
                        Some("eng"),
                        tesseract::OcrEngineMode::TesseractOnly,
                    )
                    .unwrap();
                    ts.set_variable("tessedit_pageseg_mode", "6").unwrap()
                };
                let screenshot_path = data_path.join(DATA_SCREENSHOT_DIR);

                loop {
                    let mut img = match rx.recv() {
                        Ok(x) => x,
                        Err(e) => {
                            eprintln!("Error in ocr worker: {}", e);
                            continue;
                        }
                    };

                    img = remove_not_text(&img, IMG_MAX_WHITE_DEV);
                    let mut img_path = screenshot_path.clone();
                    img_path.push(format!("{}_{}.png", unix_timestamp().unwrap(), idx));
                    img.save(&img_path).unwrap();
                    let img_path_str = format!("{:?}", img_path).replace(r#"""#, "");
                    ts = ts.set_image(&img_path_str).unwrap().recognize().unwrap();
                    let raw_ocr = ts.get_text().unwrap();
                    fs::remove_file(img_path).unwrap();
                    let closest = find_closest_levenshtein_match(&items.read().unwrap(), &raw_ocr);
                    tx.send((i, closest)).unwrap();
                }
            });
        }

        OCREngine {
            tx: [
                img_channels[0].0.clone(),
                img_channels[1].0.clone(),
                img_channels[2].0.clone(),
                img_channels[3].0.clone(),
            ],
            rx: ret_channel_rx,
        }
    }

    pub fn ocr(&self, path: &str) -> Result<Vec<(usize, ShortItem)>> {
        let img = image::open(path)?;

        for i in 0..4 {
            let cropped = img.crop_imm(
                ITEM_CROP_COORDS[i][0],
                ITEM_CROP_COORDS[i][1],
                ITEM_CROP_SIZE[0],
                ITEM_CROP_SIZE[1],
            );
            self.tx[i].send(cropped)?;
        }

        let mut results = Vec::new();

        for _ in 0..4 {
            results.push(self.rx.recv()?);
        }

        Ok(results)
    }
}

// https://github.com/WFCD/WFinfo/blob/a7d4b8311564807cf384495441a18c56f63f7eb1/WFInfo/Data.cs#L830
fn find_closest_levenshtein_match(items: &Vec<ShortItem>, target: &str) -> ShortItem {
    let mut lowest_levenshtein = 9999;
    let mut lowest_item = None;

    for item in items {
        let diff = levenshtein(target, &item.item_name);
        if diff < lowest_levenshtein {
            lowest_levenshtein = diff;
            lowest_item = Some(item);
        }
    }

    lowest_item.unwrap().clone()
}

fn remove_not_text(img: &DynamicImage, max_dev: f32) -> DynamicImage {
    let mut result = img.clone();
    for pix in img.pixels() {
        let x = pix.0;
        let y = pix.1;
        let color = pix.2;
        let hsv = to_hsv(color[0], color[1], color[2]);

        // if !in_range(hsv, (0.095 * 360.0, 0.111, 0.416), (0.15 * 360.0, 1.0, 1.0)) {
        if !in_range(hsv, (0.075 * 360.0, 0.111, 0.416), (0.35 * 360.0, 1.0, 1.0)) {
            result.put_pixel(x, y, Pixel::from_channels(0, 0, 0, 255));
        } else {
            result.put_pixel(
                x, y,
                //Pixel::from_channels(pix.2[0], pix.2[1], pix.2[2], 255),
                color,
                // Pixel::from_channels(255, 255, 255, 255),
            );
        }
    }

    result
}

fn pixel_dev(pixel: image::Rgba<u8>) -> f32 {
    (255.0 - pixel[0] as f32) + (255.0 - pixel[1] as f32) + (255.0 - pixel[2] as f32)
}

fn in_range(color: Hsv, lower: (f64, f64, f64), upper: (f64, f64, f64)) -> bool {
    // println!("{:?}", color);
    if color.h < lower.0 || color.h > upper.0 {
        return false;
    } else if color.s < lower.1 || color.s > upper.1 {
        return false;
    } else if color.v < lower.2 || color.v > upper.2 {
        return false;
    }
    true
}

fn to_hsv(r: u8, g: u8, b: u8) -> Hsv {
    let r_ = r as f64 / 255.0;
    let g_ = g as f64 / 255.0;
    let b_ = b as f64 / 255.0;

    let cmax = r_.max(g_.max(b_));
    let cmin = r_.min(g_.min(b_));
    let delta = cmax - cmin;

    let hue = 60.0
        * if delta == 0.0 {
            0.0
        } else if cmax == r_ {
            ((g_ - b_) / delta) % 6.0
        } else if cmax == g_ {
            (b_ - r_) / delta + 2.0
        } else {
            (r_ - g_) / delta + 4.0
        }
        .rem_euclid(360.0);

    let sat = if cmax == 0.0 { 0.0 } else { delta / cmax };

    let value = cmax;

    Hsv::new(hue, sat, value)
}

fn to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r_, g_, b_) = match h {
        h if h >= 0.0 && h < 60.0 => (c, x, 0.0),
        h if h >= 60.0 && h < 120.0 => (x, c, 0.0),
        h if h >= 120.0 && h < 180.0 => (0.0, c, x),
        h if h >= 180.0 && h < 240.0 => (0.0, x, c),
        h if h >= 240.0 && h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    let r = ((r_ + m) * 255.0).round() as u8;
    let g = ((g_ + m) * 255.0).round() as u8;
    let b = ((b_ + m) * 255.0).round() as u8;
    (r, g, b)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_to_hsv() {
        assert_eq!(Hsv::new(0.0, 0.0, 0.0), to_hsv(0, 0, 0));
        assert_eq!(Hsv::new(0.0, 0.0, 1.0), to_hsv(255, 255, 255));
        assert_eq!(Hsv::new(0.0, 1.0, 1.0), to_hsv(255, 0, 0));
        assert_eq!(Hsv::new(120.0, 1.0, 1.0), to_hsv(0, 255, 0));
        assert_eq!(Hsv::new(240.0, 1.0, 1.0), to_hsv(0, 0, 255));
        // assert_eq!((265.0, 0.349, 0.741), to_hsv(150, 123, 189));
    }

    #[test]
    fn test_to_rgb() {
        assert_eq!((0, 0, 0), to_rgb(0.0, 0.0, 0.0));
        assert_eq!((255, 255, 255), to_rgb(0.0, 0.0, 1.0));
        assert_eq!((255, 0, 0), to_rgb(0.0, 1.0, 1.0));
        assert_eq!((0, 255, 0), to_rgb(120.0, 1.0, 1.0));
        assert_eq!((0, 0, 255), to_rgb(240.0, 1.0, 1.0));
    }
}
