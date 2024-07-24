MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
    FLASH : ORIGIN = 0x10000100, LENGTH = 2048K - 0x2100
}
/*we now have reduced FLASH by additional 0x2000 (8K), to use to store persisting data (alarm time!)*/
/*free region:*/
/*starting address is: 0x10200100*/
/*ending address is: 0x101FE000*/