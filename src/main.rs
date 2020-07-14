#![no_std]
#![no_main]
#![feature(const_raw_ptr_to_usize_cast)]

// 80041500/44100 = 1815
// 
// 44100*160	/44100 = 160	/48000 = 147
// 44100*160*2	/44100 = 320	/48000 = 294	/96000 = 147

// pick a panicking behavior
// use panic_halt as _; // you can put a breakpoint on `rust_begin_unwind` to catch panics
// use panic_abort as _; // requires nightly
// use panic_itm as _; // logs messages over ITM; requires ITM support
use panic_semihosting as _; // logs messages to the host stderr; requires a debugger

use cortex_m::asm;
use cortex_m_rt::{entry};
use cortex_m_semihosting::hprintln;

use core::ptr;

use stm32f4::stm32f446 as pac;
use pac::{interrupt, NVIC};



static mut pacd: Option<pac::Peripherals> = None;


fn wait_for<F>(f: F) 
	where F: Fn() -> bool
{
	while !f() {}
}

const buffer_size: usize = 12;
static mut buffer1: [u16; buffer_size] = [0; buffer_size];
static mut buffer2: [u16; buffer_size] = [0; buffer_size];


#[entry]
unsafe fn main() -> ! {
	hprintln!("Entry");

	hprintln!("Disabling Interrupts");
	cortex_m::interrupt::disable();
	hprintln!("Interrupts Disabled");

	hprintln!("Unmask TIM5 and DMA2_S0 interrupt in NVIC");
	NVIC::unmask(pac::Interrupt::TIM5);
	NVIC::unmask(pac::Interrupt::DMA2_STREAM0);
	hprintln!("Done");

	while pacd.is_none() {
		pacd = pac::Peripherals::take();
	}

	let device = pacd.as_ref().unwrap();

	// Enable clock on DMA and GPIO_A
	device.RCC.ahb1enr.modify(
		|_, w| w
				//.dma1en().bit(true)
				.dma2en().bit(true)
				.gpioaen().bit(true)
	);

	// Enable clock on ADC1 and TIM5
	device.RCC.apb2enr.modify(|_, w| w.adc1en().bit(true));
	device.RCC.apb1enr.modify(|_, w| w.tim5en().bit(true));

	// Reset DMA1, DMA2, and GPIOA
	device.RCC.ahb1rstr.write(
		|w|
			w.dma2rst().bit(true).dma1rst().bit(true).gpioarst().bit(true)
	);

	// Reset ADC and TIM5
	device.RCC.apb1rstr.write(|w| w.tim5rst().bit(true));
	device.RCC.apb2rstr.write(|w| w.adcrst().bit(true));

	// Wait for reset
	for _ in 0..100 {
		asm::nop();
	}

	// Stop reseting anything on the ahb1, apb1 and apb2 bus
	device.RCC.ahb1rstr.write(|w|w.bits(0b0));
	device.RCC.apb1rstr.write(|w|w.bits(0b0));
	device.RCC.apb2rstr.write(|w|w.bits(0b0));

	// Configure GPIO_A_0 as analog
	device.GPIOA.moder.modify(
		|_, w|
			w
			.moder0().bits(0b11) // Analog Mode
			.moder1().bits(0b11) // Analog Mode
	);


	// Pointer to ADC data register
	const ADC_DR: *const u32 = 0x4001204c as *const u32;


	hprintln!("Setup dma...");

	// ## SET UP DMA ## //
	//	Set PAR
	device.DMA2.st[0].par.write(|w| w.bits(ADC_DR as u32));
	
	// 	Set destination memory buffers
	device.DMA2.st[0].m0ar.write(|w| w.bits((&buffer1 as *const u16) as u32));
	device.DMA2.st[0].m1ar.write(|w| w.bits((&buffer2 as *const u16) as u32));

	//	Set number data transer
	device.DMA2.st[0].ndtr.write(|w| w.bits(buffer_size as u32));

	// 	Set channel
	device.DMA2.st[0].cr.modify(|_, w| w.chsel().bits(0b00));

	// Set priority
	device.DMA2.st[0].cr.modify(|_, w| w.pl().bits(0b11));

	device.DMA2.st[0].fcr.modify(
		|_, w|
			w
			.fth().bits(0b01) // FIFO threshold, 01: Half Full
			.dmdis().bit(true) // Disable Direct Mode, Use FIFO
	);

	// Set rest of DMA config
	device.DMA2.st[0].cr.modify(
		|_, w|
		w
			.msize().bits(0b01) // Half Word(16 bit)
			.psize().bits(0b01) // Half Word(16 bit)
			.minc().bit(true)
			.pinc().bit(false)
			.dbm().bit(true) // Double buffer mode
			.circ().bit(true)
			.dir().bits(0b00) // Peripheral to memory
			.mburst().bits(0b00) // Single transfer
			.pburst().bits(0b00) // Single transer
			.tcie().bit(true) // Enable transfer complete interrupt
			.teie().bit(true) // Enable transfer error interrupt
	);

	// Enable DMA
	device.DMA2.st[0].cr.modify(|_, w| w.en().bit(true));

	hprintln!("Done");


	hprintln!("Setup ADC Common...");

	// ## ADC COMMON INIT ## //
	device.ADC_COMMON.ccr.modify(
		|_, w|
			w
			.adcpre().bits(0b00) // Prescalar (8) 11: PCLK2/8, 00: PCLK/2
			.multi().bits(0b00000) // Indipendent ADC mode
			.delay().bits(0b0000) // 00: 5 * adc_clk delay
	);

	hprintln!("Done");


	hprintln!("Setup sample timer (Timer 3)...");

	// ## SETUP TIMER_3 CHANNEL 1 ## //
	// Fire every 1/441000 seconds to signal the ADC to take a sample
	device.TIM5.cr1.write(
		|w|
			w
			.ckd().bits(0b00) // 00: Dont divide clock (≈80MHz ?)
			.arpe().bit(true) // auto reload
			.dir().bit(false) // count up
			.cms().bits(0b00) // Count up and down, set output when counting up
			.urs().bit(true) // Only under/over flow trigger update interrupt
			.udis().bit(false) // DO generate update events
	);

	// Output compare mode
	// Channel 1 is setup up to give a high pulse whenever
	// the COUNT of Timer1 matches a spesified calue (CCRn)
	// This channel is linked to ADC1(Through it register setup)
	// Such that it is fire whenever there is a pulse on TIMER3_CHANNEL1
	device.TIM5.ccmr1_output.write(
		|w|
			w
			.cc1s().bits(0b00) // Channel 1 is an output
			.oc1m().bits(0b011) // 001: Set on match, 011: Toggle
	);

	device.TIM5.ccer.write(
		|w|
			w
			.cc1e().bit(true) // Enable channel 1
			.cc1p().bit(false) // Channel 1 is "active high"
	);

	// Timer interrups 
	device.TIM5.dier.write(|w|w
		.uie().bit(false)
		.cc1ie().bit(true)
	);
	
	// Set auto reload register (1815 ≈ 44100hz)
	// From where does the time count down from
	// Reload Value
	device.TIM5.arr.write(|w| w.bits(0xFFFFFF as u32));

	// Start Count
	device.TIM5.cnt.write(|w| w.bits(0xFFFFFF as u32));

	// Capture/Compare value
	device.TIM5.ccr1.write(|w| w.bits(0x0 as u32));

	// Prescaler
	device.TIM5.psc.write(|w| w.bits(0x0 as u32));

	hprintln!("Done");


	hprintln!("Setup ADC1...");

	// ## ADC 1 INIT ## //
	// 1: Right Alignment
	device.ADC1.cr2.modify(|_, w| w.align().bit(false));

	device.ADC1.cr1.modify(
		|_, w|
			w
			.res().bits(0b00) // 12 bit resolution
			.scan().bit(true)
			.eocie().bit(false) // no EOC interrupt
	);

	// Set ADC sample time (Min for 12 bit resolution is 15 cycles)
	// 000: 3 cycles
	// 001: 15 cycles
	// 010: 28 cycles
	// 011: 56 cycles
	// 100: 84 cycles
	// 101: 112 cycles
	// 110: 144 cycles
	// 111: 480 cycles
	device.ADC1.smpr2.modify(
		|_, w|
			w
			.smp0().bits(0b011) // 56 cycles for some margin
			.smp1().bits(0b011) // 56 cycles for some margin
	);

	device.ADC1.cr2.modify(
		|_, w|
			w
			//.cont().bit(true) // Continous Mode
			.exten().bits(0b01) // External trigger. 01: Rising Edge, 00: No ext trigger
			.extsel().bits(0b1010) // TIM5_CH1 event
	);

	// Define sequence (Single channel)
	device.ADC1.sqr1.modify(
		|_, w|
			w.l().bits(0b0001) // Two conversions
	);

	// Set input sequence
	device.ADC1.sqr3.modify(
		|_, w|
			w.sq1().bits(0b0000) // channel 0
			.sq2().bits(0b0001) // channel 1
	);

	// Enable DMA on ADC
	device.ADC1.cr2.modify(
		|_, w|
			w
			.dma().bit(true) // Enable DMA
			.dds().bit(true) // DMA requests are issued as long as data are converted and DMA=
	);

	// Turn on the ADC
	device.ADC1.cr2.modify(
		|_, w|
			w.adon().bit(true)
			.swstart().bit(true)
	);

	hprintln!("Done");


	hprintln!("Start sample timer (TIMER 3)");
	// Enable counter
	device.TIM5.cr1.write(|w| w.cen().bit(true));
	// Reinit the counter and fire update event
	device.TIM5.egr.write(|w| w.ug().bit(true));
	hprintln!("Done");


	hprintln!("Enabling interrupts");
	cortex_m::interrupt::enable();
	hprintln!("Interrupts enabled");


	// ## DO THINGS ## //
	loop {
		asm::nop();

		// TODO: Enable ovr interrupt and move this code
		if device.ADC1.sr.read().ovr().bit() {
			device.DMA1.st[0].m0ar.write(|w| { w.bits((&buffer1 as *const u16) as u32) });
			device.DMA1.st[0].m1ar.write(|w| { w.bits((&buffer2 as *const u16) as u32) });

			//	Set number data transer
			device.DMA1.st[0].ndtr.write(|w| w.bits(buffer_size as u32));

			device.ADC1.sr.modify(|_, w| w.ovr().bit(false));

			device.ADC1.cr2.modify(
				|_, w|
					w.adon().bit(true)
			);

			device.ADC1.cr2.modify(
				|_, w|
					w.swstart().bit(true)
			);
		}
	}
}

#[interrupt]
unsafe fn TIM5() {
	hprintln!("Sample Tick");
	
	while pacd.is_none() {
		pacd = pac::Peripherals::take();
	}

	let device = pacd.as_ref().unwrap();

	hprintln!("Remove interrupt flag");
	hprintln!(
		"Filling buffer {}. I:{}",
		if device.DMA2.st[0].cr.read().ct().bit() { 1 } else { 2 },
		device.DMA2.st[0].ndtr.read().bits()
	);

	// clear update interrupt flag if set
	device.TIM5.sr.modify(|_, w| w.uif().bit(false));

	// Clear CC interrupt flag
	device.TIM5.sr.modify(|_, w| w.cc1if().bit(false));
}

#[interrupt]
unsafe fn DMA2_STREAM0() {
	while pacd.is_none() {
		pacd = pac::Peripherals::take();
	}

	let device = pacd.as_ref().unwrap();
	hprintln!("DMA Stream Full");
	device.DMA2.lifcr.write(|w| w
		.ctcif0().bit(true)
		.chtif0().bit(true)
	);
}
