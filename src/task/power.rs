//! # Power tasks
//! Determine the power state of the system: battery or power supply.
//! Determine the supply voltage of the system.
//!
//! ## Noise Filtering Strategy
//!
//! This module implements software filtering to compensate for
//! voltage dividers (220kΩ/100kΩ) that may be susceptible to electrical noise, especially
//! from `NeoPixel` switching and EMI in the enclosure environment.
//!
//! ### USB Power Detection (VBUS)
//! - **Debouncing**: Requires multiple consecutive identical readings before confirming a state change
//! - **Delay sampling**: Samples are spaced apart to avoid transient noise
//! - **State tracking**: Only updates when state is confirmed different from last known state
//!
//! ### Voltage Measurement (VSYS)
//! - **Median filtering**: Takes multiple ADC samples and computes median to reject outlier spikes
//! - **Settling time**: Allows time between samples for high-impedance source to settle
//! - **Hysteresis**: Event handler ignores small voltage changes (<0.1V) to prevent flicker

use defmt::info;
use embassy_futures::select::select;
use embassy_rp::{
    adc::{Adc, Channel},
    gpio::Input,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use moving_median::MovingMedian;

use crate::event::{Event, send_event};

/// Number of consecutive readings required to confirm a USB state change
/// Higher values = more noise immunity but slower response
const VBUS_DEBOUNCE_COUNT: u8 = 5;

/// Delay between debounce samples (milliseconds)
/// Allows transients to settle between readings
const VBUS_DEBOUNCE_DELAY_MS: u64 = 10;

/// Number of ADC samples to median for voltage reading, use odd number to avoid averaging calculation
const VSYS_SAMPLE_COUNT: usize = 9;

/// Delay between ADC samples (milliseconds)
/// Allows high-impedance source to settle on ADC input capacitance
const VSYS_SAMPLE_DELAY_MS: u64 = 5;

/// Signal for waking the vsys voltage reader early
static VSYS_WAKE_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// Signals the vsys voltage reader to wake up early
pub fn signal_vsys_wake() {
    VSYS_WAKE_SIGNAL.signal(());
}

/// Waits for the vsys wake signal
async fn wait_for_vsys_wake() {
    VSYS_WAKE_SIGNAL.wait().await;
}

/// determine the power source of the system, specifically if the USB power supply is connected
/// the USB power supply is connected, if the pin is high
/// Note: We are using a voltage divider to detect the USB power supply through a GPIO pin. Due to the intricacies of the Pico W,
/// the VBUS pin is not available for direct use (it is run through the wifi module, and there is no safe way to use wifi and the
/// vbus concurrently).
///
/// This implementation uses debouncing and filtering to handle noise from high-impedance voltage divider
/// and EMI from `NeoPixels` and other switching loads.
#[embassy_executor::task]
pub async fn usb_power_detector(mut vbus_in: Input<'static>) {
    info!("usb_power task started");

    // wait for the system to settle, before starting the loop -> the vbus_in pin is not stable immediately
    Timer::after(Duration::from_secs(1)).await;

    // Track the last confirmed state
    let mut last_confirmed_state = vbus_in.is_high();
    send_event(Event::Vbus(last_confirmed_state)).await;

    loop {
        // Wait for any edge detection
        vbus_in.wait_for_any_edge().await;

        // Debounce: read the pin multiple times to confirm the state change
        let mut consecutive_count = 0u8;
        let potential_new_state = vbus_in.is_high();

        // Only proceed if this is different from our last confirmed state
        if potential_new_state != last_confirmed_state {
            // Sample the pin multiple times with delays
            for _ in 0..VBUS_DEBOUNCE_COUNT {
                Timer::after(Duration::from_millis(VBUS_DEBOUNCE_DELAY_MS)).await;

                if vbus_in.is_high() == potential_new_state {
                    consecutive_count += 1;
                } else {
                    // State is unstable, break early
                    break;
                }
            }

            // Only update if all samples confirmed the new state
            if consecutive_count == VBUS_DEBOUNCE_COUNT {
                last_confirmed_state = potential_new_state;
                info!("USB power state confirmed: {}", potential_new_state);
                send_event(Event::Vbus(last_confirmed_state)).await;
            } else {
                info!(
                    "USB power state change rejected (noise): {} rejected samples",
                    VBUS_DEBOUNCE_COUNT - consecutive_count
                );
            }
        }
    }
}

/// measure the voltage of the Vsys rail
/// this is either the battery voltage or the usb power supply voltage, if the usb power supply is connected.
/// Note: We are using a voltage divider to measure the Vsys voltage through a GPIO pin. Due to the intricacies of the Pico W,
/// the VSYS pin is not available for direct use (it is run through the wifi module, and there is no safe way to use wifi and the
/// vsys concurrently).
///
/// This implementation uses multiple samples and median filtering to filter noise from high-impedance circuit
/// and switching noise from `NeoPixels` and other loads. Median filtering is superior to averaging for
/// rejecting impulse noise spikes from `NeoPixel` switching.
#[embassy_executor::task]
pub async fn vsys_voltage_reader(mut adc: Adc<'static, embassy_rp::adc::Async>, mut channel: Channel<'static>) {
    info!("vsys_voltage task started");

    let downtime = Duration::from_secs(600); // 10 minutes

    loop {
        // wait for the system to settle, before reading -> the adc is not stable immediately if we got here after either the usb power was cut the system just started
        Timer::after(Duration::from_secs(1)).await;

        // Create median filter for this measurement cycle
        // Using u16 since ADC values are 12-bit (0-4095)
        let mut median_filter = MovingMedian::<u16, VSYS_SAMPLE_COUNT>::new();
        let mut valid_samples = 0;

        // Collect samples into median filter
        for _ in 0..VSYS_SAMPLE_COUNT {
            if let Ok(adc_value) = adc.read(&mut channel).await {
                median_filter.add_value(adc_value);
                valid_samples += 1;
            }
            // Small delay between samples to allow settling time for high-impedance source
            Timer::after(Duration::from_millis(VSYS_SAMPLE_DELAY_MS)).await;
        }

        if valid_samples > 0 {
            // Get median value (outlier spikes automatically rejected!)
            let median_adc_value = median_filter.median();

            // reference voltage is 3.3V, and the voltage divider ratio is 3.2 (R10: 220kΩ + R9: 100kΩ) / 100kΩ. The ADC is 12-bit, so 2^12 = 4096
            let voltage = f32::from(median_adc_value) * 3.3 * 3.2 / 4096.0;

            info!(
                "Vsys voltage reading: {}V (median filtered from {} samples)",
                voltage, valid_samples
            );
            send_event(Event::Vsys(voltage)).await;
        } else {
            info!("Vsys voltage reading failed: no valid samples");
        }

        // we either wait for the downtime or until we are woken up early. Whatever comes first, starts the next iteration.
        let downtime_timer = Timer::after(downtime);
        select(downtime_timer, wait_for_vsys_wake()).await;
    }
}
