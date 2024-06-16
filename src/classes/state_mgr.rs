// not sure if this will even be a thing... but it's here for now
// This is a class that will be used to manage the state of the system, I will need something to call functions across the system and that gets messy quickly
// This will be a way to keep things organized
// Actually I guess I need some sort of event system to call functions across the system... I'll have to look into that but Rust does not seem to have a built in event system
// For now the idea is to do a small version of dependenbcy injection and pass this class around to the other classes that need it; none of that yet implemented

use defmt::*;

pub trait StateManagement {
    fn menu_press_green_button(&mut self);
    fn menu_press_blue_button(&mut self);
    fn menu_press_yellow_button(&mut self);
    fn log_emit(&mut self, message: &str, source: &str);
}

pub struct StateManager;

impl StateManagement for StateManager {
    fn menu_press_green_button(&mut self) {
        info!("Green button pressed")
    }

    fn menu_press_blue_button(&mut self) {
        info!("Blue button pressed")
    }

    fn menu_press_yellow_button(&mut self) {
        info!("Yellow button pressed")
    }

    fn log_emit(&mut self, message: &str, source: &str) {
        info!("{}: {}", source, message)
    }
}
