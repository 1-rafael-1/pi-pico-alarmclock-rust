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
