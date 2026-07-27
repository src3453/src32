// Expose the main components of the CPT32 emulator as a library

pub mod cpu;
pub mod bus;
pub mod sys;
pub mod devices {
    pub mod ram;
    pub mod vdp {
        pub mod clut;
        pub mod vdp;
        pub mod gp;
        pub mod pcg;
        pub mod reg;
    }
    pub mod pec {
        pub mod serial;
    }
}