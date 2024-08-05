//! # Button Tasks
//! This module contains the tasks for the buttons. Each button has its own task.

use crate::task::resources::{BlueButtonResources, GreenButtonResources, YellowButtonResources};
use crate::task::state::{Events, EVENT_CHANNEL};
use defmt::info;
use defmt::Format;
use embassy_executor::Spawner;
use embassy_rp::gpio::{self, Input, Level};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Sender;
use embassy_time::{with_deadline, Duration, Instant, Timer};
use {defmt_rtt as _, panic_probe as _};

// Button Manager
// Handles button press, hold, and long hold
// Debounces button press
pub struct ButtonManager<'a> {
    input: Input<'a>,
    debounce_duration: Duration,
    events: Events,
    button: Button,
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
            debounce_duration: Duration::from_millis(100), // hardcoding, all buttons have the same debounce duration
            events,
            button,
            sender,
        }
    }

    pub async fn handle_button_press(&mut self) {
        loop {
            // button pressed
            let _debounce = self.debounce().await;
            let start = Instant::now();
            info!("{} Press", self.events);

            // send button press to channel -> this is a ToDo, we will want to do this reacting to the length of the press somehow
            // maybe we need a pipe instead of a channel to stream the button presses to the orchestrator
            let presses: u32 = 0;
            let event = match self.events {
                Events::BlueBtn(0) => Events::BlueBtn(presses + 1),
                Events::GreenBtn(0) => Events::GreenBtn(presses + 1),
                Events::YellowBtn(0) => Events::YellowBtn(presses + 1),
                _ => panic!("Invalid Event"),
            };
            self.sender.send(event).await;

            match with_deadline(start + Duration::from_secs(1), self.debounce()).await {
                // Button Released < 1s
                Ok(_) => {
                    info!("pressed for: {}ms", start.elapsed().as_millis());
                    continue;
                }
                // button held for > 1s
                Err(_) => {
                    info!("Held");
                }
            }

            match with_deadline(start + Duration::from_secs(5), self.debounce()).await {
                // Button released <5s
                Ok(_) => {
                    info!("pressed for: {}ms", start.elapsed().as_millis());
                    continue;
                }
                // button held for > >5s
                Err(_) => {
                    info!("Long Held");
                }
            }

            // wait for button release before handling another press
            self.debounce().await;
            info!("pressed for: {}ms", start.elapsed().as_millis());
        }
    }

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
pub async fn green_button(_spawner: Spawner, r: GreenButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = EVENT_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, Events::GreenBtn(0), Button::Green, sender);
    info!("{} task started", btn.button);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn blue_button(_spawner: Spawner, r: BlueButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = EVENT_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, Events::BlueBtn(0), Button::Blue, sender);
    info!("{} task started", btn.button);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn yellow_button(_spawner: Spawner, r: YellowButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = EVENT_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, Events::YellowBtn(0), Button::Yellow, sender);
    info!("{} task started", btn.button);
    btn.handle_button_press().await;
}
