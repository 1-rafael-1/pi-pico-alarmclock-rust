//! # Button Tasks
//! This module contains the tasks for the buttons. Each button has its own task.

use crate::event::{Event, send_event};
use defmt::{Format, info};
use embassy_rp::gpio::{Input, Level};
use embassy_time::{Duration, Instant, Timer, with_deadline};
use {defmt_rtt as _, panic_probe as _};

/// Handles button press, hold, and long hold
/// Debounces button press
pub struct ButtonManager<'a> {
    /// The input pin for the button
    input: Input<'a>,
    /// The debounce duration
    debounce_duration: Duration,
    /// The event to send when the button is pressed or held
    event: Event,
    /// The button being managed
    button: Button,
    /// The interval between hold events
    hold_event_interval: Duration,
}

/// The buttons of the system
#[derive(Debug, Format, Eq, PartialEq, Clone)]
pub enum Button {
    /// No button
    None,
    /// Green button
    Green,
    /// Blue button
    Blue,
    /// Yellow button
    Yellow,
}

impl<'a> ButtonManager<'a> {
    /// Create a new `ButtonManager`
    pub const fn new(input: Input<'a>, event: Event, button: Button) -> Self {
        Self {
            input,
            debounce_duration: Duration::from_millis(80), // hardcoding, all buttons have the same debounce duration
            event,
            button,
            hold_event_interval: Duration::from_millis(150), // hardcoding, all buttons have the same hold event interval
        }
    }

    /// Handle the button press event. This function is an infinite loop that waits for a debounced button press event, then determines if the button was pressed or held.
    /// The most important thing to know here is that a basic button event is either the button being pressed or being released, both of which are a change in the input level that we track.
    pub async fn handle_button_press(&mut self) {
        'mainloop: loop {
            // we do nothing, until we have a debounced button event, either changing from high to low or low to high. Here at this point we expect the level to be low, normally.
            // The button is normally high, and when pressed, it goes low. So we wait for the button to be pressed.

            let init_level = self.debounce().await;
            // if the button is not pressed, we continue with the main loop
            if init_level != Level::Low {
                continue 'mainloop;
            }

            // we wait for the button to be released, depending on how fast that happens, we have a one-time press event or a hold.
            let level_result =
                with_deadline(Instant::now() + Duration::from_secs(1), self.debounce()).await;

            // Button Released < 1s -> we have a one-time press event
            if let Ok(level) = level_result {
                // if the button is released, we send one press event down the channel
                if level == Level::High {
                    send_event(self.event.clone()).await;
                }
                // and then we continue with the main loop
                continue 'mainloop;
            }

            // button held for > 1s
            // not a one-time press event, but a hold event
            // we have a button being held, we need to handle the hold event.
            'holding: loop {
                // we wait for either the button to change its level or the hold event interval to expire
                let level_result = with_deadline(
                    Instant::now() + self.hold_event_interval,
                    self.input.wait_for_any_edge(),
                )
                .await;

                if level_result.is_ok() {
                    // if the button level changed, we break the loop and continue with the main loop and send no event
                    break 'holding;
                }

                // Timeout occurred - check if button is still held
                if self.input.get_level() == Level::High {
                    // if the button is released, we continue with the main loop and send no event
                    continue 'mainloop;
                }

                // if the button is still held, we send an event down the channel, and then return to the beginning of the loop
                send_event(self.event.clone()).await;
            }
        }
    }

    /// Debounce the button press by waiting for the button to be stable for a given duration. We determine the input level, then await any edge,
    /// then wait for the debounce duration, then check if the input level has changed. If it has, we break the loop and return the new level.
    pub async fn debounce(&mut self) -> Level {
        loop {
            let l1 = self.input.get_level();

            self.input.wait_for_any_edge().await;

            Timer::after(self.debounce_duration).await;

            let l2 = self.input.get_level();
            if l1 != l2 {
                break l2;
            }
        }
    }
}

#[embassy_executor::task(pool_size = 3)]
pub async fn button_handler(input: Input<'static>, event: Event, button: Button) {
    let mut btn = ButtonManager::new(input, event, button);
    info!("{} task started", btn.button);
    btn.handle_button_press().await;
}
