//! # Main
//! This is the main entry point of the program.
//! we are in an environment with constrained resources, so we do not use the standard library and we define a different entry point.
#![no_std]
#![no_main]

use crate::task::alarm_settings::alarm_settings_handler;
use crate::task::alarm_trigger::alarm_trigger_task;
use crate::task::buttons::{Button, button_handler};
use crate::task::display::display_handler;
use crate::task::light_effects::light_effects_handler;
use crate::task::orchestrate::{alarm_expirer, orchestrator, scheduler};
use crate::task::power::{usb_power_detector, vsys_voltage_reader};
use crate::task::sound::sound_handler;
use crate::task::task_messages::Events;
use crate::task::time_updater::time_updater;
use defmt::info;
use embassy_executor::{Spawner, main};
use embassy_rp::adc::{Adc, Channel, Config as AdcConfig, InterruptHandler as AdcInterruptHandler};
use embassy_rp::bind_interrupts;
use embassy_rp::clocks::{ClockConfig, CoreVoltage};
use embassy_rp::config::Config;
use embassy_rp::flash::{Async, Flash};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::i2c::{Config as I2cConfig, I2c, InterruptHandler as I2cInterruptHandler};
use embassy_rp::peripherals::{I2C0, PIO0, UART1};
use embassy_rp::pio::InterruptHandler as PioInterruptHandler;
use embassy_rp::rtc::{InterruptHandler as RtcInterruptHandler, Rtc};
use embassy_rp::spi::{Config as SpiConfig, Phase, Polarity, Spi};
use embassy_rp::uart::{BufferedInterruptHandler, BufferedUart, Config as UartConfig};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

// import the task module (submodule of src)
mod task;

// import the utility module (submodule of src)
mod utility;

// Bind the interrupts on a global scope for convenience
bind_interrupts!(pub struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
    UART1_IRQ => BufferedInterruptHandler<UART1>;
    ADC_IRQ_FIFO => AdcInterruptHandler;
    RTC_IRQ => RtcInterruptHandler;
});

/// Helper function to spawn tasks and unwrap, panicking if spawn fails.
/// This is acceptable during initialization as we want to fail fast if we can't spawn a task.
#[allow(clippy::unwrap_used)]
fn spawn_unwrap<S>(
    spawner: Spawner,
    token: Result<embassy_executor::SpawnToken<S>, embassy_executor::SpawnError>,
) {
    spawner.spawn(token.unwrap());
}

/// The main entry point of the program. This is where the tasks are spawned and run.
#[main]
async fn main(spawner: Spawner) {
    info!("Program start");

    // Initialize the peripherals for the RP2040, use reduced clock settings for lower power consumption
    #[allow(clippy::unwrap_used)]
    let mut clock_config = ClockConfig::system_freq(18_000_000).unwrap();
    clock_config.core_voltage = CoreVoltage::V0_90;
    let config = Config::new(clock_config);
    let p = embassy_rp::init(config);

    // Orchestrator tasks
    spawn_unwrap(spawner, orchestrator());
    spawn_unwrap(spawner, scheduler());
    spawn_unwrap(spawner, alarm_expirer());
    spawn_unwrap(spawner, alarm_trigger_task());

    // Green button
    let btn_green = Input::new(p.PIN_20, Pull::Up);
    spawn_unwrap(
        spawner,
        button_handler(btn_green, Events::GreenBtn, Button::Green),
    );

    // Blue button
    let btn_blue = Input::new(p.PIN_21, Pull::Up);
    spawn_unwrap(
        spawner,
        button_handler(btn_blue, Events::BlueBtn, Button::Blue),
    );

    // Yellow button
    let btn_yellow = Input::new(p.PIN_22, Pull::Up);
    spawn_unwrap(
        spawner,
        button_handler(btn_yellow, Events::YellowBtn, Button::Yellow),
    );

    // USB power detector
    let vbus_in = Input::new(p.PIN_28, Pull::None);
    spawn_unwrap(spawner, usb_power_detector(vbus_in));

    // Vsys voltage reader
    let adc = Adc::new(p.ADC, Irqs, AdcConfig::default());
    let vsys_channel = Channel::new_pin(p.PIN_27, Pull::None);
    spawn_unwrap(spawner, vsys_voltage_reader(adc, vsys_channel));

    // Display
    let mut i2c_config = I2cConfig::default();
    i2c_config.frequency = 400_000;
    let i2c = I2c::new_async(p.I2C0, p.PIN_13, p.PIN_12, Irqs, i2c_config);
    spawn_unwrap(spawner, display_handler(i2c));

    // DFPlayer sound handler
    let mut uart_config = UartConfig::default();
    uart_config.baudrate = 9600;
    static TX_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
    static RX_BUFFER: StaticCell<[u8; 256]> = StaticCell::new();
    let tx_buf = TX_BUFFER.init([0u8; 256]);
    let rx_buf = RX_BUFFER.init([0u8; 256]);
    let uart = BufferedUart::new(p.UART1, p.PIN_4, p.PIN_5, Irqs, tx_buf, rx_buf, uart_config);
    let dfplayer_pwr = Output::new(p.PIN_8, Level::Low);
    spawn_unwrap(spawner, sound_handler(uart, dfplayer_pwr));

    // Alarm settings persistence
    const FLASH_SIZE: usize = 2 * 1024 * 1024;
    let flash = Flash::<_, Async, FLASH_SIZE>::new(p.FLASH, p.DMA_CH4);
    spawn_unwrap(spawner, alarm_settings_handler(flash));

    // Time updater with WiFi and RTC
    let rtc = Rtc::new(p.RTC, Irqs);
    let wifi_peripherals = crate::task::time_updater::WifiPeripherals {
        pwr_pin: p.PIN_23,
        cs_pin: p.PIN_25,
        pio: p.PIO0,
        dio_pin: p.PIN_24,
        clk_pin: p.PIN_29,
        dma_ch: p.DMA_CH0,
    };
    spawn_unwrap(spawner, time_updater(spawner, rtc, wifi_peripherals));

    // Neopixel light effects
    let mut spi_config = SpiConfig::default();
    spi_config.frequency = 3_800_000;
    spi_config.phase = Phase::CaptureOnFirstTransition;
    spi_config.polarity = Polarity::IdleLow;
    let spi = Spi::new_txonly(p.SPI0, p.PIN_18, p.PIN_19, p.DMA_CH1, spi_config);
    spawn_unwrap(spawner, light_effects_handler(spi));
}
