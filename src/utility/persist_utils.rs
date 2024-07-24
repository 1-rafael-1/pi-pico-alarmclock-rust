use sequential_storage;
use sequential_storage::map;

pub async fn test() {
    //let mut flash = init_flash();
    // -> fnd something like this let mut flash = embassy_embedded_hal::adapter::BlockingAsync::new(flash);
    // in the examples or the appropriate library
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
