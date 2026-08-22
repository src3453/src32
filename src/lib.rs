// Expose the main components of the CPT32 emulator as a library

pub mod bus;
pub mod cpu;
pub mod sys;
pub mod devices {
    pub mod cpu;
    pub mod ram;
    pub mod irqc {
        pub mod irqc;
    }
    pub mod sgu {
        pub mod s3w2;
        pub mod sgu;
    }
    pub mod vdp {
        pub mod clut;
        pub mod gp;
        pub mod pcg;
        pub mod reg;
        pub mod vdp;
    }
    pub mod pec {
        pub mod rng;
        pub mod serial;
    }
}
