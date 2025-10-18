//! # Power tasks
//! Determine the power state of the system: battery or power supply.
//! Detremine the supply voltage of the system.

use crate::task::task_messages::{EVENT_CHANNEL, Events, VSYS_WAKE_SIGNAL};
use defmt::info;
use embassy_futures::select::select;
use embassy_rp::adc::{Adc, Channel};
use embassy_rp::gpio::Input;
use embassy_time::{Duration, Timer};

/// determine the power source of the system, specifically if the USB power supply is connected
/// the USB power supply is connected, if the pin is high
/// Note: We are using a voltage divider to detect the USB power supply through a GPIO pin. Due to the intricacies of the Pico W,
/// the VBUS pin is not available for direct use (it is run through the wifi module, and there is no safe way to use wifi and the
/// vbus concurrently).
#[embassy_executor::task]
pub async fn usb_power_detector(mut vbus_in: Input<'static>) {
    info!("usb_power task started");
    let sender = EVENT_CHANNEL.sender();

    // wait for the system to settle, before starting the loop -> the vbus_in pin is not stable immediately
    Timer::after(Duration::from_secs(1)).await;

    loop {
        sender.send(Events::Vbus(vbus_in.is_high())).await;
        vbus_in.wait_for_any_edge().await;
    }
}

/// measure the voltage of the Vsys rail
/// this is either the battery voltage or the usb power supply voltage, if the usb power supply is connected.
/// Note: We are using a voltage divider to measure the Vsys voltage through a GPIO pin. Due to the intricacies of the Pico W,
/// the VSYS pin is not available for direct use (it is run through the wifi module, and there is no safe way to use wifi and the
/// vsys concurrently).
#[embassy_executor::task]
pub async fn vsys_voltage_reader(
    mut adc: Adc<'static, embassy_rp::adc::Async>,
    mut channel: Channel<'static>,
) {
    info!("vsys_voltage task started");

    let sender = EVENT_CHANNEL.sender();
    let downtime = Duration::from_secs(600); // 10 minutes

    loop {
        // wait for the system to settle, before reading -> the adc is not stable immediately if we got here after either the usb power was cut the system just started
        Timer::after(Duration::from_secs(1)).await;
        // read the adc value
        if let Ok(adc_value) = adc.read(&mut channel).await {
            // reference voltage is 3.3V, and the voltage divider ratio is 2.65. The ADC is 12-bit, so 2^12 = 4096
            let voltage = (f32::from(adc_value)) * 3.3 * 2.65 / 4096.0;
            sender.send(Events::Vsys(voltage)).await;

            // we either wait for the downtime or until we are woken up early. Whatever comes first, starts the next iteration.
            let downtime_timer = Timer::after(downtime);
            select(downtime_timer, VSYS_WAKE_SIGNAL.wait()).await;
        }
    }
}
