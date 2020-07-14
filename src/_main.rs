#![no_std]
#![no_main]
#![feature(alloc_error_handler)]
#![feature(const_raw_ptr_to_usize_cast)]


extern crate device;
extern crate alloc;
use core::alloc::{Layout};
use alloc_cortex_m::{CortexMHeap};

use device::interrupt;

// pick a panicking behavior
// use panic_halt as _; // you can put a breakpoint on `rust_begin_unwind` to catch panics
// use panic_abort as _; // requires nightly
// use panic_itm as _; // logs messages over ITM; requires ITM support
use panic_semihosting as _; // logs messages to the host stderr; requires a debugger

use cortex_m::asm;
use cortex_m_rt::entry;
use cortex_m_semihosting::hprintln;

use embedded_hal::timer::Cancel;
use hal::timer;
use hal::timer::Timer;
use nb;
use stm32f4xx_hal as hal;
use core::ptr;
use crate::hal::{prelude::*, stm32};

use stm32f4xx_hal::{
    prelude::*,
    pwm,
    gpio::gpioa,
    adc::{
        Adc,
        config::AdcConfig,
        config::SampleTime,
        config::Sequence,
        config::Eoc,
        config::Scan,
        config::Continuous,
        config::Clock,
        config::Dma
    },
};

#[global_allocator]
static ALLOCATOR: CortexMHeap = CortexMHeap::empty();


// Size of the heap in bytes
const SIZE: usize = 1024;

extern "C" {
    static mut __sheap: u8;
}


#[alloc_error_handler]
fn alloc_err(layout: Layout) -> ! {
    hprintln!("Oh oh, memory error").unwrap();
    loop {}
}


#[entry]
unsafe fn main() -> ! {
    let rcc: *mut u32 = 0x40023800 as *mut u32;

    let ADC_SR: *mut u32 = 0x40012000 as *mut u32;
    let ADC_CR1: *mut u32 = 0x40012004 as *mut u32;
    let ADC_CR2: *mut u32 = 0x40012008 as *mut u32;

    // Enable clock for DMA1
    ptr::write_volatile(
        rcc.offset(12),
        *(rcc.offset(12)) | (0b1 << 21)
    );

    let __sheap_ref: *const u8 = &__sheap as *const u8;
    ALLOCATOR.init(__sheap_ref as usize, 1024);
    hprintln!("Hello, world!").unwrap();


    if let Some(device) = stm32::Peripherals::take() {
        let config = AdcConfig::default()
            //.continuous(Continuous::Continuous)
            .dma(Dma::Continuous)
            .scan(Scan::Enabled)
            .clock(Clock::Pclk2_div_8);

        let mut adc = Adc::adc1(device.ADC1, true, config);
    
        let gpioa = device.GPIOA.split();
        
        let pa0 = gpioa.pa0.into_analog();
        //let pa1 = gpioa.pa1.into_analog();
    
        adc.configure_channel(&pa0, Sequence::One, SampleTime::Cycles_112);
        //adc.configure_channel(&pa1, Sequence::Two, SampleTime::Cycles_15);


        let dma1: *mut u32 = hal::stm32::DMA1::ptr() as *mut _;

        // Res [31..28] Channel [27..25] MBURST[24..23] PBURST[22..21] RES CT DB PL [17..16]
        // PINCOS 2*MSIZE 2*PSIZE MINC PINC CIRC DIR PFCTRL TCIE HTIE TEIE DMEIE EN
        
        let channel : u32 = 0b000 << 25; // 0th channel
        let pl : u32 = 0b11 << 16; // Priority
        let pincos : u32 = 0b0 << 15; // Peripheral increment offset size
        let msize : u32 = 0b01 << 13; // Memory transfer size. 00=1byte, 01=2byte, 10=4byte, 11=res
        let psize : u32 = 0b01 << 11; // Peripheral data size = 2byte=16bit, same as msize
        let minc : u32 = 0b1 << 10; // Memory increment
        let pinc : u32 = 0b0 << 9; // No peripheral increment
        let circ : u32 = 0b1 << 8; // Circular buffer
        let dir : u32 = 0b00 << 6;  // 00: peripheral-to-memory 01: memory-to-peripheral 10: memory-to-memory 11: reserved
        let pfctrl : u32 = 0b0 << 5; // DMA is flow controller
        let tcie : u32 = 0b0 << 4;
        let htie : u32 = 0b0 << 3;
        let teie : u32 = 0b0 << 2;
        let en : u32 = 0b1 << 0; // Enable dma channel

        let DMA_LISR = dma1;
        let DMA_HISR = dma1.offset(1);
        let DMA_LIFCR = dma1.offset(2);
        let DMA_HIFCR = dma1.offset(3);
        let DMA_S0CR = dma1.offset(4);
        let DMA_S0NDTR = dma1.offset(5);
        let DMA_S0PAR = dma1.offset(6);
        let DMA_S0M0AR = dma1.offset(7);
        let DMA_S0M1AR = dma1.offset(8);
        let DMA_S0FCR = dma1.offset(9);
        let ADC_DR = adc.data_register_address();

        // Set DMA peripheral address
        ptr::write_volatile(DMA_S0PAR, ADC_DR);

        // Create buffer and set DMA destination address
        let buffer_size: u16 = 32;
        // Set memory address (Create buffer)
        let buffer: *mut u16 = alloc::alloc::alloc(
            Layout::array::<u16>(buffer_size as usize).unwrap()
        ) as *mut u16;
        ptr::write_volatile(DMA_S0M0AR, buffer as u32);

        // Set read count to buffer size
        ptr::write_volatile(DMA_S0NDTR, buffer_size as u32);

        // Set Channel
        ptr::write_volatile(DMA_S0CR, 
            channel
        );

        // Set Priority (0)
        ptr::write_volatile(DMA_S0CR, 
            pl
        );

        // Disable FIFO
        ptr::write_volatile(DMA_S0FCR, 0x00000000);

        ptr::write_volatile(DMA_S0CR, 
            dir | pinc | minc | psize | msize | circ
        );

        // and enable DMA
        ptr::write_volatile(DMA_S0CR, 
            *(DMA_S0CR as *const u32) | 0b1
            as u32
        );

        //let mut adc2d;
        adc.start_conversion();

        let mut data: u16;
        loop {
            data = adc.current_sample();
        }
    
        // if let Some(dp) = stm32::Peripherals::take() {
        //     // Set up the system clock.
        //     let rcc = device.RCC.constrain();
        //     let clocks = rcc.cfgr.freeze();
        
        //     let channels = (
        //         gpioa.pa8.into_alternate_af1(),
        //         gpioa.pa9.into_alternate_af1(),
        //     );
        
        //     let pwm = pwm::tim1(device.TIM1, channels, clocks, 20u32.khz());
        //     let (mut ch1, mut ch2) = pwm;
        //     let max_duty = ch1.get_max_duty();
    
        //     ch1.set_duty(max_duty / 2);
            
        //     ch1.enable();
        // }
    }


    loop {
        // your code goes here
    }
}
