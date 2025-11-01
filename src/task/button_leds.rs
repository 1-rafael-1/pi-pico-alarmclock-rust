//! # Button LEDs task
//! This module contains the task that controls the button LED ring lights.
//!
//! The task is responsible for controlling the button LED ring lights via an IRLZ44N MOSFET on GPIO 26.
use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_rp::gpio::Output;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};

/// Button LED timeout duration
const BUTTON_LED_TIMEOUT: Duration = Duration::from_secs(10);

/// Command for controlling the button LEDs
#[derive(Clone, Copy)]
pub enum ButtonLedCommand {
    /// Turn on the button LEDs
    On,
    /// Turn off the button LEDs
    Off,
    /// Turn on the button LEDs with a 10-second timeout before turning off
    OnWithTimeout,
}

/// Signal for controlling the button LEDs
static BUTTON_LEDS_SIGNAL: Signal<CriticalSectionRawMutex, ButtonLedCommand> = Signal::new();

/// Signals the button LEDs task with a command
pub fn signal_button_leds(command: ButtonLedCommand) {
    BUTTON_LEDS_SIGNAL.signal(command);
}

/// Waits for the next button LEDs command
async fn wait_for_button_leds_command() -> ButtonLedCommand {
    BUTTON_LEDS_SIGNAL.wait().await
}

#[embassy_executor::task]
pub async fn button_leds_handler(mut control_pin: Output<'static>) {
    info!("Button LEDs task started");

    // Initially turn off the LEDs
    control_pin.set_low();

    loop {
        // Wait for the next command
        let command = wait_for_button_leds_command().await;

        match command {
            ButtonLedCommand::On => {
                info!("Turning on button LEDs");
                control_pin.set_high();
            }
            ButtonLedCommand::Off => {
                info!("Turning off button LEDs");
                control_pin.set_low();
            }
            ButtonLedCommand::OnWithTimeout => {
                info!("Turning on button LEDs with timeout");
                control_pin.set_high();

                // Wait for either timeout to pass or a new command
                'timeout_loop: loop {
                    let result = select(Timer::after(BUTTON_LED_TIMEOUT), wait_for_button_leds_command()).await;

                    match result {
                        Either::First(()) => {
                            // Timeout elapsed, turn off LEDs
                            info!("Button LED timeout elapsed");
                            control_pin.set_low();
                            break 'timeout_loop;
                        }
                        Either::Second(new_command) => {
                            match new_command {
                                ButtonLedCommand::OnWithTimeout => {
                                    // Reset the timer, continue the timeout loop
                                    info!("Button LED timeout reset");
                                    // Continue the loop to wait another timeout period
                                }
                                ButtonLedCommand::On => {
                                    // Switch to permanent on mode
                                    info!("Button LEDs switching to permanent on");
                                    control_pin.set_high();
                                    break 'timeout_loop;
                                }
                                ButtonLedCommand::Off => {
                                    // Turn off immediately
                                    info!("Turning off button LEDs");
                                    control_pin.set_low();
                                    break 'timeout_loop;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
