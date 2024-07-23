/// This module contains the task that displays information on the OLED display.
///
/// The task is responsible for initializing the display, displaying images and text, and updating the display.
use crate::task::resources::{DisplayResources, Irqs};
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::i2c::{Config, I2c};
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    image::Image,
    mono_font::{ascii::FONT_9X18_BOLD, MonoTextStyleBuilder},
    pixelcolor::{BinaryColor, Gray8},
    prelude::*,
    text::{Baseline, Text},
};
use ssd1306_async::{prelude::*, I2CDisplayInterface, Ssd1306};
use tinybmp::Bmp;

struct Bmps {
    saber: Bmp<'static, Gray8>,
    colon: Bmp<'static, Gray8>,
    _0: Bmp<'static, Gray8>,
    _1: Bmp<'static, Gray8>,
    _2: Bmp<'static, Gray8>,
    _3: Bmp<'static, Gray8>,
    _4: Bmp<'static, Gray8>,
    _5: Bmp<'static, Gray8>,
    _6: Bmp<'static, Gray8>,
    _7: Bmp<'static, Gray8>,
    _8: Bmp<'static, Gray8>,
    _9: Bmp<'static, Gray8>,
    bat_000: Bmp<'static, Gray8>,
    bat_020: Bmp<'static, Gray8>,
    bat_040: Bmp<'static, Gray8>,
    bat_060: Bmp<'static, Gray8>,
    bat_080: Bmp<'static, Gray8>,
    bat_100: Bmp<'static, Gray8>,
    bat_mains: Bmp<'static, Gray8>,
    settings: Bmp<'static, Gray8>,
}

impl Bmps {
    fn new() -> Self {
        Self {
            saber: Bmp::from_slice(include_bytes!("../../media/saber.bmp"))
                .expect("Failed to load BMP image"),
            colon: Bmp::from_slice(include_bytes!("../../media/colon.bmp"))
                .expect("Failed to load BMP image"),
            _0: Bmp::from_slice(include_bytes!("../../media/0.bmp"))
                .expect("Failed to load BMP image"),
            _1: Bmp::from_slice(include_bytes!("../../media/1.bmp"))
                .expect("Failed to load BMP image"),
            _2: Bmp::from_slice(include_bytes!("../../media/2.bmp"))
                .expect("Failed to load BMP image"),
            _3: Bmp::from_slice(include_bytes!("../../media/3.bmp"))
                .expect("Failed to load BMP image"),
            _4: Bmp::from_slice(include_bytes!("../../media/4.bmp"))
                .expect("Failed to load BMP image"),
            _5: Bmp::from_slice(include_bytes!("../../media/5.bmp"))
                .expect("Failed to load BMP image"),
            _6: Bmp::from_slice(include_bytes!("../../media/6.bmp"))
                .expect("Failed to load BMP image"),
            _7: Bmp::from_slice(include_bytes!("../../media/7.bmp"))
                .expect("Failed to load BMP image"),
            _8: Bmp::from_slice(include_bytes!("../../media/8.bmp"))
                .expect("Failed to load BMP image"),
            _9: Bmp::from_slice(include_bytes!("../../media/9.bmp"))
                .expect("Failed to load BMP image"),
            bat_000: Bmp::from_slice(include_bytes!("../../media/bat_000.bmp"))
                .expect("Failed to load BMP image"),
            bat_020: Bmp::from_slice(include_bytes!("../../media/bat_020.bmp"))
                .expect("Failed to load BMP image"),
            bat_040: Bmp::from_slice(include_bytes!("../../media/bat_040.bmp"))
                .expect("Failed to load BMP image"),
            bat_060: Bmp::from_slice(include_bytes!("../../media/bat_060.bmp"))
                .expect("Failed to load BMP image"),
            bat_080: Bmp::from_slice(include_bytes!("../../media/bat_080.bmp"))
                .expect("Failed to load BMP image"),
            bat_100: Bmp::from_slice(include_bytes!("../../media/bat_100.bmp"))
                .expect("Failed to load BMP image"),
            bat_mains: Bmp::from_slice(include_bytes!("../../media/bat_mains.bmp"))
                .expect("Failed to load BMP image"),
            settings: Bmp::from_slice(include_bytes!("../../media/settings.bmp"))
                .expect("Failed to load BMP image"),
        }
    }
}

struct Images<'a> {
    saber: Image<'a, Bmp<'static, Gray8>>,
    colon: Image<'a, Bmp<'static, Gray8>>,
    _0: Image<'a, Bmp<'static, Gray8>>,
    _1: Image<'a, Bmp<'static, Gray8>>,
    _2: Image<'a, Bmp<'static, Gray8>>,
    _3: Image<'a, Bmp<'static, Gray8>>,
    _4: Image<'a, Bmp<'static, Gray8>>,
    _5: Image<'a, Bmp<'static, Gray8>>,
    _6: Image<'a, Bmp<'static, Gray8>>,
    _7: Image<'a, Bmp<'static, Gray8>>,
    _8: Image<'a, Bmp<'static, Gray8>>,
    _9: Image<'a, Bmp<'static, Gray8>>,
    bat_000: Image<'a, Bmp<'static, Gray8>>,
    bat_020: Image<'a, Bmp<'static, Gray8>>,
    bat_040: Image<'a, Bmp<'static, Gray8>>,
    bat_060: Image<'a, Bmp<'static, Gray8>>,
    bat_080: Image<'a, Bmp<'static, Gray8>>,
    bat_100: Image<'a, Bmp<'static, Gray8>>,
    bat_mains: Image<'a, Bmp<'static, Gray8>>,
    settings: Image<'a, Bmp<'static, Gray8>>,
}

impl<'a> Images<'a> {
    fn new(bmps: &'a Bmps) -> Self {
        Self {
            saber: Image::new(&bmps.saber, Point::new(0, 0)),
            colon: Image::new(&bmps.colon, Point::new(0, 0)),
            _0: Image::new(&bmps._0, Point::new(0, 0)),
            _1: Image::new(&bmps._1, Point::new(0, 0)),
            _2: Image::new(&bmps._2, Point::new(0, 0)),
            _3: Image::new(&bmps._3, Point::new(0, 0)),
            _4: Image::new(&bmps._4, Point::new(0, 0)),
            _5: Image::new(&bmps._5, Point::new(0, 0)),
            _6: Image::new(&bmps._6, Point::new(0, 0)),
            _7: Image::new(&bmps._7, Point::new(0, 0)),
            _8: Image::new(&bmps._8, Point::new(0, 0)),
            _9: Image::new(&bmps._9, Point::new(0, 0)),
            bat_000: Image::new(&bmps.bat_000, Point::new(108, 0)),
            bat_020: Image::new(&bmps.bat_020, Point::new(108, 0)),
            bat_040: Image::new(&bmps.bat_040, Point::new(108, 0)),
            bat_060: Image::new(&bmps.bat_060, Point::new(108, 0)),
            bat_080: Image::new(&bmps.bat_080, Point::new(108, 0)),
            bat_100: Image::new(&bmps.bat_100, Point::new(108, 0)),
            bat_mains: Image::new(&bmps.bat_mains, Point::new(108, 0)),
            settings: Image::new(&bmps.settings, Point::new(0, 0)),
        }
    }

    fn repositon_image(&mut self, bmps: &'a Bmps, image_name: &str, new_position: Point) {
        match image_name {
            "saber" => self.saber = Image::new(&bmps.saber, new_position),
            "colon" => self.colon = Image::new(&bmps.colon, new_position),
            "_0" => self._0 = Image::new(&bmps._0, new_position),
            "_1" => self._1 = Image::new(&bmps._1, new_position),
            "_2" => self._2 = Image::new(&bmps._2, new_position),
            "_3" => self._3 = Image::new(&bmps._3, new_position),
            "_4" => self._4 = Image::new(&bmps._4, new_position),
            "_5" => self._5 = Image::new(&bmps._5, new_position),
            "_6" => self._6 = Image::new(&bmps._6, new_position),
            "_7" => self._7 = Image::new(&bmps._7, new_position),
            "_8" => self._8 = Image::new(&bmps._8, new_position),
            "_9" => self._9 = Image::new(&bmps._9, new_position),
            "bat_000" => self.bat_000 = Image::new(&bmps.bat_000, new_position),
            "bat_020" => self.bat_020 = Image::new(&bmps.bat_020, new_position),
            "bat_040" => self.bat_040 = Image::new(&bmps.bat_040, new_position),
            "bat_060" => self.bat_060 = Image::new(&bmps.bat_060, new_position),
            "bat_080" => self.bat_080 = Image::new(&bmps.bat_080, new_position),
            "bat_100" => self.bat_100 = Image::new(&bmps.bat_100, new_position),
            "bat_mains" => self.bat_mains = Image::new(&bmps.bat_mains, new_position),
            "settings" => self.settings = Image::new(&bmps.settings, new_position),
            _ => self::panic!("Unknown image name: {}", image_name),
        }
    }
}

#[embassy_executor::task]
pub async fn display(_spawner: Spawner, r: DisplayResources) {
    info!("Display task started");

    let scl = r.scl;
    let sda = r.sda;
    let mut config = Config::default();
    config.frequency = 400_000;
    let i2c = I2c::new_async(r.i2c0, scl, sda, Irqs, config);

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    match display.init().await {
        Ok(_) => {}
        Err(e) => {
            error!("Failed to initialize display: {}", defmt::Debug2Format(&e));
            return;
        }
    }
    display.set_brightness(Brightness::DIMMEST).await.unwrap();

    // Load BMP images from media
    let bmps = Bmps::new();
    // Create images from BMPs
    let mut images = Images::new(&bmps);

    loop {
        Timer::after(Duration::from_millis(1_000)).await;

        display.clear();

        let text_style = MonoTextStyleBuilder::new()
            .font(&FONT_9X18_BOLD)
            .text_color(BinaryColor::On)
            .build();
        Text::with_baseline("Text 1", Point::zero(), text_style, Baseline::Top)
            .draw(&mut display)
            .unwrap();
        Text::with_baseline("Text 2", Point::new(0, 16), text_style, Baseline::Top)
            .draw(&mut display)
            .unwrap();

        display.flush().await.unwrap();

        Timer::after(Duration::from_millis(1_000)).await;

        display.clear();
        images.saber.draw(&mut display.color_converted()).unwrap();
        display.flush().await.unwrap();

        Timer::after(Duration::from_millis(1_000)).await;

        display.clear();
        images.repositon_image(&bmps, "saber", Point::new(0, 50));
        images.saber.draw(&mut display.color_converted()).unwrap();
        display.flush().await.unwrap();
        images.repositon_image(&bmps, "saber", Point::new(0, 0));

        Timer::after(Duration::from_millis(1_000)).await;
    }
}
