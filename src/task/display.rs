//! # Display task
//! This module contains the task that displays information on the OLED display.
//!
//! The task is responsible for initializing the display, displaying images and text, and updating the display.
use crate::state::{BatteryLevel, OperationMode, SYSTEM_STATE};
use crate::task::buttons::Button;
use crate::task::time_updater::RTC_MUTEX;
use crate::task::watchdog::{TaskId, report_task_success};
use crate::utility::string_utils::StringUtils;
use core::fmt::Write;
use defmt::{Debug2Format, info, warn};
use embassy_rp::i2c::{Async, I2c};
use embassy_rp::peripherals::I2C0;
use embassy_rp::rtc::{DateTime, DayOfWeek};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use embedded_graphics::{
    image::Image,
    mono_font::{
        MonoTextStyle, MonoTextStyleBuilder,
        ascii::{FONT_6X13, FONT_8X13_BOLD},
    },
    pixelcolor::{BinaryColor, Gray8},
    prelude::*,
    text::{Baseline, Text},
};
use heapless::String;
use ssd1306_async::{I2CDisplayInterface, Ssd1306, prelude::*};
use tinybmp::Bmp;

/// Signal for triggering display updates
static DISPLAY_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Triggers a display update
pub fn signal_display_update() {
    DISPLAY_SIGNAL.signal(());
}

/// Waits for the next display update signal
async fn wait_for_display_update() {
    DISPLAY_SIGNAL.wait().await;
}

/// Loads and holds BMP images and Points for the display
/// Holds some settings for composing the display
struct Settings<'a> {
    /// BMP image of the saber icon
    saber: Bmp<'static, Gray8>,
    /// BMP image of the colon icon
    colon: Bmp<'static, Gray8>,
    /// BMP images of the digits 0-9
    digits: [Bmp<'static, Gray8>; 10],
    /// BMP images of the battery status icons
    bat: [Bmp<'static, Gray8>; 6],
    /// BMP image of the battery mains icon
    bat_mains: Bmp<'static, Gray8>,
    /// BMP image of the settings icon
    setup: Bmp<'static, Gray8>,
    /// Position of the state indicator images, hight is 16
    state_indicator_position: Point,
    /// Position of the battery status images, hight is 11
    bat_position: Point,
    /// (Starting) Position of the time digits, hight is 24
    time_digit_start_position: Point,
    /// Position of the date text
    date_position: Point,
    /// (Starting) Position of content
    content_start_position: Point,
    /// Style of the state area text
    state_indicator_text_style: MonoTextStyle<'a, BinaryColor>,
    /// Style of the date text
    date_text_style: MonoTextStyle<'a, BinaryColor>,
    /// Style of the menu and system info content text
    content_text_style: MonoTextStyle<'a, BinaryColor>,
}

impl Settings<'_> {
    /// Creates a new Settings struct, loading all BMP images from the media folder
    #[allow(clippy::expect_used, clippy::too_many_lines)]
    fn new() -> Self {
        Self {
            saber: Bmp::from_slice(include_bytes!("../media/saber.bmp")).unwrap_or_else(|_| {
                warn!("Failed to load saber.bmp, using fallback");
                Bmp::from_slice(include_bytes!("../media/0.bmp"))
                    .expect("Fallback 0.bmp image must be available")
            }),
            colon: Bmp::from_slice(include_bytes!("../media/colon.bmp")).unwrap_or_else(|_| {
                warn!("Failed to load colon.bmp, using fallback");
                Bmp::from_slice(include_bytes!("../media/0.bmp"))
                    .expect("Fallback 0.bmp image must be available")
            }),
            digits: [
                Bmp::from_slice(include_bytes!("../media/0.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 0.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/1.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 1.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/2.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 2.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/3.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 3.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/4.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 4.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/5.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 5.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/6.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 6.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/7.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 7.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/8.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 8.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/9.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load 9.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
            ],
            bat: [
                Bmp::from_slice(include_bytes!("../media/bat_000.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load bat_000.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/bat_020.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load bat_020.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/bat_040.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load bat_040.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/bat_060.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load bat_060.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/bat_080.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load bat_080.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
                Bmp::from_slice(include_bytes!("../media/bat_100.bmp")).unwrap_or_else(|_| {
                    warn!("Failed to load bat_100.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                }),
            ],
            bat_mains: Bmp::from_slice(include_bytes!("../media/bat_mains.bmp")).unwrap_or_else(
                |_| {
                    warn!("Failed to load bat_mains.bmp, using fallback");
                    Bmp::from_slice(include_bytes!("../media/0.bmp"))
                        .expect("Fallback 0.bmp image must be available")
                },
            ),
            setup: Bmp::from_slice(include_bytes!("../media/settings.bmp")).unwrap_or_else(|_| {
                warn!("Failed to load settings.bmp, using fallback");
                Bmp::from_slice(include_bytes!("../media/0.bmp"))
                    .expect("Fallback 0.bmp image must be available")
            }),
            state_indicator_position: Point::new(0, 0),
            bat_position: Point::new(108, 0),
            time_digit_start_position: Point::new(13, 21),
            content_start_position: Point::new(0, 19),
            date_position: Point::new(0, 51),
            state_indicator_text_style: MonoTextStyleBuilder::new()
                .font(&FONT_8X13_BOLD)
                .text_color(BinaryColor::On)
                .build(),
            date_text_style: MonoTextStyleBuilder::new()
                .font(&FONT_6X13)
                .text_color(BinaryColor::On)
                .build(),
            content_text_style: MonoTextStyleBuilder::new()
                .font(&FONT_6X13)
                .text_color(BinaryColor::On)
                .build(),
        }
    }
}

/// Draws the state indicator in the top-left area of the display
fn draw_state_indicator<D>(
    display: &mut D,
    operation_mode: &OperationMode,
    alarm_enabled: bool,
    settings: &Settings,
) where
    D: embedded_graphics::draw_target::DrawTarget<Color = BinaryColor>,
{
    match operation_mode {
        OperationMode::Normal => {
            if alarm_enabled {
                let saber = Image::new(&settings.saber, settings.state_indicator_position);
                let _ = saber.draw(&mut display.color_converted());
            }
        }
        OperationMode::SetAlarmTime => {
            let setup_img = Image::new(&settings.setup, settings.state_indicator_position);
            let _ = setup_img.draw(&mut display.color_converted());
        }
        OperationMode::Menu => {
            let _ = Text::with_baseline(
                "Menu",
                settings.state_indicator_position,
                settings.state_indicator_text_style,
                Baseline::Top,
            )
            .draw(display);
        }
        OperationMode::SystemInfo => {
            let _ = Text::with_baseline(
                "Sys.-Info",
                settings.state_indicator_position,
                settings.state_indicator_text_style,
                Baseline::Top,
            )
            .draw(display);
        }
        OperationMode::Alarm | OperationMode::Standby => {
            // Button info is drawn separately in alarm mode - this is handled in main content
            // Nothing shown for standby mode
        }
    }
}

/// Draws the battery status indicator in the top-right area of the display
fn draw_battery_status<D>(display: &mut D, battery_level: &BatteryLevel, settings: &Settings)
where
    D: embedded_graphics::draw_target::DrawTarget<Color = BinaryColor>,
{
    let bat_image: Image<Bmp<'static, Gray8>> = match battery_level {
        BatteryLevel::Bat000 => Image::new(&settings.bat[0], settings.bat_position),
        BatteryLevel::Bat020 => Image::new(&settings.bat[1], settings.bat_position),
        BatteryLevel::Bat040 => Image::new(&settings.bat[2], settings.bat_position),
        BatteryLevel::Bat060 => Image::new(&settings.bat[3], settings.bat_position),
        BatteryLevel::Bat080 => Image::new(&settings.bat[4], settings.bat_position),
        BatteryLevel::Bat100 => Image::new(&settings.bat[5], settings.bat_position),
        BatteryLevel::Charging => Image::new(&settings.bat_mains, settings.bat_position),
    };
    let _ = bat_image.draw(&mut display.color_converted());
}

/// Draws the time display in the center area of the display
fn draw_time_display<D>(display: &mut D, hours: u8, minutes: u8, settings: &Settings)
where
    D: embedded_graphics::draw_target::DrawTarget<Color = BinaryColor>,
{
    let mut digit_next_position = settings.time_digit_start_position;

    let first_hour_digit = Image::new(&settings.digits[(hours / 10) as usize], digit_next_position);
    digit_next_position.x += 24;

    let second_hour_digit =
        Image::new(&settings.digits[(hours % 10) as usize], digit_next_position);
    digit_next_position.x += 24;

    let colon = Image::new(&settings.colon, digit_next_position);
    digit_next_position.x += 11;

    let first_minute_digit = Image::new(
        &settings.digits[(minutes / 10) as usize],
        digit_next_position,
    );
    digit_next_position.x += 24;

    let second_minute_digit = Image::new(
        &settings.digits[(minutes % 10) as usize],
        digit_next_position,
    );

    let _ = first_hour_digit.draw(&mut display.color_converted());
    let _ = second_hour_digit.draw(&mut display.color_converted());
    let _ = colon.draw(&mut display.color_converted());
    let _ = first_minute_digit.draw(&mut display.color_converted());
    let _ = second_minute_digit.draw(&mut display.color_converted());
}

/// Draws the menu content in the center area of the display
fn draw_menu_content<D>(display: &mut D, settings: &Settings)
where
    D: embedded_graphics::draw_target::DrawTarget<Color = BinaryColor>,
{
    let mut content_next_position = settings.content_start_position;
    let _ = Text::with_baseline(
        "Green: Sys. Info",
        content_next_position,
        settings.content_text_style,
        Baseline::Top,
    )
    .draw(display);
    content_next_position.y += 15;
    let _ = Text::with_baseline(
        "Blue: Standby",
        content_next_position,
        settings.content_text_style,
        Baseline::Top,
    )
    .draw(display);
    content_next_position.y += 15;
    let _ = Text::with_baseline(
        "Yellow: Back",
        content_next_position,
        settings.content_text_style,
        Baseline::Top,
    )
    .draw(display);
}

/// Draws the system info content in the center area of the display
fn draw_system_info_content<D>(
    display: &mut D,
    vsys: f32,
    usb_power: bool,
    upper: f32,
    lower: f32,
    settings: &Settings,
) where
    D: embedded_graphics::draw_target::DrawTarget<Color = BinaryColor>,
{
    let mut content_next_position = settings.content_start_position;

    let mut vsys_txt: String<20> = String::new();
    let _ = write!(vsys_txt, "Vsys:  {vsys}V");
    let _ = Text::with_baseline(
        &vsys_txt,
        content_next_position,
        settings.content_text_style,
        Baseline::Top,
    )
    .draw(display);
    content_next_position.y += 15;

    let mut usb_txt: String<20> = String::new();
    let _ = write!(usb_txt, "USB:   {usb_power}");
    let _ = Text::with_baseline(
        &usb_txt,
        content_next_position,
        settings.content_text_style,
        Baseline::Top,
    )
    .draw(display);
    content_next_position.y += 15;

    let mut bounds_txt: String<20> = String::new();
    let _ = write!(bounds_txt, "Upper/Lower {upper}/{lower}V");
    let _ = Text::with_baseline(
        &bounds_txt,
        content_next_position,
        settings.content_text_style,
        Baseline::Top,
    )
    .draw(display);
}

/// Draws the alarm button prompt in the state indicator area
fn draw_alarm_button_prompt<D>(display: &mut D, button: &Button, settings: &Settings)
where
    D: embedded_graphics::draw_target::DrawTarget<Color = BinaryColor>,
{
    let mut btn_txt: String<13> = String::new();
    let _ = write!(btn_txt, "Press {button:?}!");
    let _ = Text::with_baseline(
        &btn_txt,
        settings.state_indicator_position,
        settings.state_indicator_text_style,
        Baseline::Top,
    )
    .draw(display);
}

/// Draws the date text at the bottom of the display
fn draw_date<D>(display: &mut D, dt: &DateTime, settings: &Settings)
where
    D: embedded_graphics::draw_target::DrawTarget<Color = BinaryColor>,
{
    let date = StringUtils::convert_datetime_to_str(dt);
    let _ = Text::with_baseline(
        &date,
        settings.date_position,
        settings.date_text_style,
        Baseline::Top,
    )
    .draw(display);
}

#[embassy_executor::task]
#[allow(clippy::too_many_lines)]
pub async fn display_handler(i2c: I2c<'static, I2C0, Async>) {
    info!("Display task started");

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    if let Err(e) = display.init().await {
        warn!("Failed to initialize display: {}", defmt::Debug2Format(&e));
        return;
    }

    let _ = display.set_brightness(Brightness::DIMMEST).await;

    let settings = Settings::new();

    'mainloop: loop {
        // Wait for a signal to update the display
        wait_for_display_update().await;

        // get the current time out of the mutex and quickly drop the mutex
        let dt: DateTime = {
            let rtc_guard = RTC_MUTEX.lock().await;
            let Some(rtc) = rtc_guard.as_ref() else {
                warn!("RTC not initialized");
                drop(rtc_guard);
                Timer::after(Duration::from_secs(1)).await;
                continue 'mainloop;
            };
            match rtc.now() {
                Ok(dt) => dt,
                Err(e) => {
                    info!("RTC not running: {:?}", Debug2Format(&e));
                    // Return an empty DateTime
                    DateTime {
                        year: 0,
                        month: 0,
                        day: 0,
                        day_of_week: DayOfWeek::Monday,
                        hour: 0,
                        minute: 0,
                        second: 0,
                    }
                }
            }
        };

        // get the state of the system out of the mutex and quickly drop the mutex
        let system_state_guard = SYSTEM_STATE.lock().await;
        let Some(system_state) = system_state_guard.clone() else {
            warn!("System state not initialized");
            drop(system_state_guard);
            Timer::after(Duration::from_secs(1)).await;
            continue 'mainloop;
        };

        // Store operation mode locally to avoid move issues
        let operation_mode = system_state.operation_mode.clone();

        // prepare the display, note that nothing is sent to the display before flush()
        display.clear();

        // Draw state indicator (or alarm button prompt)
        if operation_mode == OperationMode::Alarm {
            let btn = system_state
                .alarm_settings
                .get_first_valid_stop_alarm_button();
            draw_alarm_button_prompt(&mut display, &btn, &settings);
        } else {
            draw_state_indicator(
                &mut display,
                &operation_mode,
                system_state.alarm_settings.get_enabled(),
                &settings,
            );
        }

        // Draw battery status
        draw_battery_status(
            &mut display,
            &system_state.power_state.get_battery_level(),
            &settings,
        );

        // Draw main content (time or menu)
        let (hours, minutes) = match operation_mode {
            OperationMode::Normal | OperationMode::Alarm => (dt.hour, dt.minute),
            OperationMode::SetAlarmTime => (
                system_state.alarm_settings.get_hour(),
                system_state.alarm_settings.get_minute(),
            ),
            _ => (0, 0),
        };

        match operation_mode {
            OperationMode::Normal | OperationMode::Alarm | OperationMode::SetAlarmTime => {
                // Display the time
                draw_time_display(&mut display, hours, minutes, &settings);
            }
            OperationMode::Menu => {
                draw_menu_content(&mut display, &settings);
            }
            OperationMode::SystemInfo => {
                let vsys = system_state.power_state.get_vsys();
                let usb_power = system_state.power_state.get_usb_power();
                let upper = system_state.power_state.get_battery_voltage_fully_charged();
                let lower = system_state.power_state.get_battery_voltage_empty();

                draw_system_info_content(&mut display, vsys, usb_power, upper, lower, &settings);
            }
            OperationMode::Standby => {
                let _ = Text::with_baseline(
                    "Going to sleep...",
                    settings.content_start_position,
                    settings.content_text_style,
                    Baseline::Top,
                )
                .draw(&mut display);
                let _ = display.flush().await;
                Timer::after(Duration::from_secs(5)).await;
                display.clear();
                let _ = display.flush().await;
            }
        }

        // Draw date (if in normal/alarm mode)
        if matches!(operation_mode, OperationMode::Normal | OperationMode::Alarm) {
            draw_date(&mut display, &dt, &settings);
        }

        // finally: send the display buffer to the display and we are done for this cycle
        let _ = display.flush().await;

        // Report successful display update to watchdog
        report_task_success(TaskId::Display).await;
    }
}
