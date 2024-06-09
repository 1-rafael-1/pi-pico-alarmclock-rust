#![no_std]
#![no_main]

use defmt_rtt as _; // global logger
use panic_probe as _;

#[defmt_test::tests]
mod validate_ci {
    #[init]
    fn init() {
        // Initialization code here
    }

    #[test]
    fn test_something() {
        // Your test code here
    }
}
