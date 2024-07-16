use crate::task::resources::{BlueButtonResources, GreenButtonResources, YellowButtonResources};
use crate::task::state::{BLUE_BTN_CHANNEL, GREEN_BTN_CHANNEL, YELLOW_BTN_CHANNEL};
use defmt::info;
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
    id: &'a str,
    sender: Sender<'a, CriticalSectionRawMutex, u8, 1>,
}

impl<'a> ButtonManager<'a> {
    pub fn new(
        input: Input<'a>,
        id: &'a str,
        sender: Sender<'a, CriticalSectionRawMutex, u8, 1>,
    ) -> Self {
        Self {
            input,
            debounce_duration: Duration::from_millis(100), // hardcoding, all buttons have the same debounce duration
            id,
            sender,
        }
    }

    pub async fn handle_button_press(&mut self) {
        loop {
            // button pressed
            let _debounce = self.debounce().await;
            let start = Instant::now();
            info!("{} Press", self.id);

            // send button press to channel -> this is a ToDo, we will want to do this reacting to the length of the press somehow
            // maybe we need a pipe instead of a channel to stream the button presses to the orchestrator
            self.sender.send(1).await;

            match with_deadline(start + Duration::from_secs(1), self.debounce()).await {
                // Button Released < 1s
                Ok(_) => {
                    info!("{} pressed for: {}ms", self.id, start.elapsed().as_millis());
                    continue;
                }
                // button held for > 1s
                Err(_) => {
                    info!("{} Held", self.id);
                }
            }

            match with_deadline(start + Duration::from_secs(5), self.debounce()).await {
                // Button released <5s
                Ok(_) => {
                    info!("{} pressed for: {}ms", self.id, start.elapsed().as_millis());
                    continue;
                }
                // button held for > >5s
                Err(_) => {
                    info!("{} Long Held", self.id);
                }
            }

            // wait for button release before handling another press
            self.debounce().await;
            info!("{} pressed for: {}ms", self.id, start.elapsed().as_millis());
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
    let sender = GREEN_BTN_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, "green_button", sender);
    info!("{} task started", btn.id);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn blue_button(_spawner: Spawner, r: BlueButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = BLUE_BTN_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, "blue_button", sender);
    info!("{} task started", btn.id);
    btn.handle_button_press().await;
}

#[embassy_executor::task]
pub async fn yellow_button(_spawner: Spawner, r: YellowButtonResources) {
    let input = gpio::Input::new(r.button_pin, gpio::Pull::Up);
    let sender = YELLOW_BTN_CHANNEL.sender();
    let mut btn = ButtonManager::new(input, "yellow_button", sender);
    info!("{} task started", btn.id);
    btn.handle_button_press().await;
}
