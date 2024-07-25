use crate::task::resources::FlashResources;
use core::ops::Range;
use embassy_rp::flash::Async;
use embassy_rp::flash::Flash;
use embassy_rp::peripherals::FLASH;
use sequential_storage;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{fetch_item, store_item};

const FLASH_SIZE: usize = 2 * 1024 * 1024;

pub struct PersistedData<'a> {
    flash: Flash<'a, FLASH, Async, { FLASH_SIZE }>,
    flash_range: Range<u32>,
    data_buffer: [u8; 128],
}

impl<'a> PersistedData<'a> {
    fn new(r: FlashResources) -> Self {
        let mut flash = Flash::<_, Async, { FLASH_SIZE }>::new(r.flash, r.dma_ch);
        Self {
            flash_range: 0x1FDF01..0x1FFFFF,
            data_buffer: [0; 128],
            flash,
        }
    }

    async fn fetch_alarm_time(&mut self) -> (u8, u8) {
        let hour = fetch_item(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &0,
        )
        .await
        .unwrap()
        .unwrap();
        let minute = fetch_item(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &1,
        )
        .await
        .unwrap()
        .unwrap();
        (hour, minute)
    }

    async fn store_alarm_time(&mut self, alarmtime: (u8, u8)) {
        // hour
        let key: u8 = 0;
        let value: u8 = alarmtime.0;
        store_item(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
            &value,
        )
        .await
        .unwrap();
        //minute
        let key: u8 = 1;
        let value: u8 = alarmtime.1;
        store_item(
            &mut self.flash,
            self.flash_range.clone(),
            &mut NoCache::new(),
            &mut self.data_buffer,
            &key,
            &value,
        )
        .await
        .unwrap();
    }
}
