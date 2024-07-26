MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
    FLASH : ORIGIN = 0x10000100, LENGTH = 2020K 
    /* original wozuld have been: 2048K - 0x100, but we reserve some space at the end to persist data for the alarm time */
}
