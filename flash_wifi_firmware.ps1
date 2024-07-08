# Navigate to the cyw43-firmware directory
Set-Location -Path "C:\Users\rafae\git\pi-pico-alarmclock-rust\wifi-firmware\cyw43-firmware"

# Download firmware using probe-rs
Write-Host "Downloading firmware..."
probe-rs download 43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
probe-rs download 43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
Write-Host "Firmware download completed."