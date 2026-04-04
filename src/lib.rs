pub mod cpu;
pub mod bus;
pub mod devices {
    pub mod ram;
    pub mod vdp {
        pub mod vdp;
        pub mod gp;
        pub mod reg;
    }
}