use embassy_rp::peripherals::PIO0;
use embassy_rp::pio::InterruptHandler;

pub struct Irqs {
    pub pio0_irq_0: InterruptHandler<PIO0>,
}
