//! # Button Tasks
//! This module contains the tasks for the buttons. Each button has its own task.

use crate::task::resources::{BlueButtonResources, GreenButtonResources, YellowButtonResources};
use crate::task::task_messages::{Events, EVENT_CHANNEL};
use defmt::{info, Format};
use embassy_executor::Spawner;
use embassy_rp::gpio::{self, Input, Level};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Sender;
use embassy_time::{with_deadline, Duration, Instant, Timer};
use {defmt_rtt as _, panic_probe as _};

/// Button Manager
/// Handles button press, hold, and long hold
/// Debounces button press
pub struct ButtonManager<'a> {
    input: Input<'a>,
    debounce_duration: Duration,
    events: Events,
    button: Button,
    hold_event_interval: Duration,
    sender: Sender<'a, CriticalSectionRawMutex, Events, 10>,
}

/// The buttons of the system
#[derive(Debug, Format, PartialEq, Clone)]
pub enum Button {
    None,
    Green,
    Blue,
    Yellow,
}

impl<'a> ButtonManager<'a> {
    pub fn new(
        input: Input<'a>,
        events: Events,
        button: Button,
        sender: Sender<'a, CriticalSectionRawMutex, Events, 10>,
    ) -> Self {
        Self {
            input,
            debounce_duration: Duration::from_millis(80), // hardcoding, all buttons have the same debounce duration
            events,
            button,
            sender,
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
            };

            // we wait for the button to be released, depending on how fast that happens, we have a one-time press event or a hold.
            let level_result =
                with_deadline(Instant::now() + Duration::from_secs(1), self.debounce()).await;
            match level_result {
                // Button Released < 1s -> we have a one-time press event
                Ok(level) => {
                    // if the button is not released, we continue with the main loop
                    if level != Level::High {
                        continue 'mainloop;
                    } else {
                        // we send one press event down the channel
                        let event = match self.events {
                            Events::BlueBtn => Events::BlueBtn,
                            Events::GreenBtn => Events::GreenBtn,
                            Events::YellowBtn => Events::YellowBtn,
                            _ => panic!("Invalid Event"),
                        };
                        self.sender.send(event).await;
                        // and then we continue with the main loop
                        continue 'mainloop;
                    }
                }
                // button held for > 1s
                // not a one-time press event, but a hold event
                Err(_) => {
                    // here we do nothing and leave this pattern matching block
                }
            };

            // we have a button being held, we need to handle the hold event.
            'holding: loop {
                // we wait for either the button to change its level or the hold event interval to expire
                let level_result = with_deadline(
                    Instant::now() + self.hold_event_interval,
                    self.input.wait_for_any_edge(),
                )
                .await;
                match level_result {
                    Ok(_) => {
                        // if the button level changed, we break the loop and continue with the main loop and send no event
                        break 'holding;
                    }
                    Err(_) => {
                        if self.input.get_level() == Level::High {
                            // if the button is released, we continue with the main loop and send no event
                            continue 'mainloop;
                        } else {
                            // if the button is still held, we send an event down the channel, and then return to the beginning of the loop
                            let event = match self.events {
                                Events::BlueBtn => Events::BlueBtn,
                                Events::GreenBtn => Events::GreenBtn,
                                Events::YellowBtn => Events::YellowBtn,
                                _ => panic!("Invalid Event"),
                            };
                            self.sender.send(event).await;
                            continue 'holding;
                        }
                    }
                }
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

#[embassy_executor::task]
pub async fn green_button_handler(_spawner: Spawner, r: GreenButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = EVENT_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, Events::GreenBtn, Button::Green, sender);
    info!("{} task started", btn.button);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn blue_button_handler(_spawner: Spawner, r: BlueButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = EVENT_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, Events::BlueBtn, Button::Blue, sender);
    info!("{} task started", btn.button);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn yellow_button_handler(_spawner: Spawner, r: YellowButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = EVENT_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, Events::YellowBtn, Button::Yellow, sender);
    info!("{} task started", btn.button);
    btn.handle_button_press().await;
}
