use crate::task::resources::FlashResources;
use core::ops::Range;
use embassy_rp::flash::Async;
use embassy_rp::flash::Flash;
use embassy_rp::peripherals::FLASH;
use sequential_storage;
use sequential_storage::cache::NoCache;
use sequential_storage::map::{fetch_item, store_item};

const FLASH_SIZE: usize = 2 * 1024 * 1024;

struct PersistedData<'a> {
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

    async fn store_alarm_time(&mut self, hour: u8, minute: u8) {
        // hour
        let key: u8 = 0;
        let value: u8 = hour;
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
        let value: u8 = minute;
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

// // Initialize the flash. This can be internal or external
// let mut flash = init_flash();
// // These are the flash addresses in which the crate will operate.
// // The crate will not read, write or erase outside of this range.
// let flash_range = 0x1000..0x3000;
// // We need to give the crate a buffer to work with.
// // It must be big enough to serialize the biggest value of your storage type in,
// // rounded up to to word alignment of the flash. Some kinds of internal flash may require
// // this buffer to be aligned in RAM as well.
// let mut data_buffer = [0; 128];

// // We can fetch an item from the flash. We're using `u8` as our key type and `u32` as our value type.
// // Nothing is stored in it yet, so it will return None.

// assert_eq!(
//     fetch_item::<u8, u32, _>(
//         &mut flash,
//         flash_range.clone(),
//         &mut NoCache::new(),
//         &mut data_buffer,
//         &42,
//     ).await.unwrap(),
//     None
// );

// // Now we store an item the flash with key 42.
// // Again we make sure we pass the correct key and value types, u8 and u32.
// // It is important to do this consistently.

// store_item(
//     &mut flash,
//     flash_range.clone(),
//     &mut NoCache::new(),
//     &mut data_buffer,
//     &42u8,
//     &104729u32,
// ).await.unwrap();

// // When we ask for key 42, we not get back a Some with the correct value

// assert_eq!(
//     fetch_item::<u8, u32, _>(
//         &mut flash,
//         flash_range.clone(),
//         &mut NoCache::new(),
//         &mut data_buffer,
//         &42,
//     ).await.unwrap(),
//     Some(104729)
// );
