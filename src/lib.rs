// Expose the main components of the CPT32 emulator as a library

pub mod bus;
pub mod cpu;
pub mod sys;
pub mod music;
pub mod wav;
pub mod devices {
    pub mod bmc;
    pub mod cpu;
    pub mod dmac {
        pub mod dmac;
    }
    pub mod ram;
    pub mod irqc {
        pub mod irqc;
    }
    pub mod sgu {
        pub mod s3w2;
        pub mod sgu;
        pub mod sqv4;
    }
    pub mod vdp {
        pub mod chr_rom;
        pub mod clut;
        pub mod compositor;
        pub mod gp;
        pub mod pcg;
        pub mod reg;
        pub mod vdp;
    }
    pub mod sgc {
        pub mod sgc;
    }
    pub mod vpu {
        pub mod vpu;
    }
    pub mod pec {
        pub mod pec;
        pub mod rng;
        pub mod serial;
        pub mod idc {
            pub mod idc;
        }
    }
}
