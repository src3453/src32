use cpt32::bus::Bus;
use cpt32::devices::ram::connect_ram;
use cpt32::wax::{load_bytes, parse};

const TEST_WAX: &[u8] = include_bytes!("../examples/test.wax");
const TEXT_BLOCK: &[u8] = b"W/S DIST A/D SPD\0J/K R U/I G N/M B\0D:\0S:\0R:\0G:\0B:\0";

#[test]
fn example_wax_restores_string_table_at_guest_addresses_used_by_code() {
    let image = parse(TEST_WAX).unwrap();
    assert_eq!(image.base, 0);
    assert_eq!(image.entry, 0);
    let text_block = image
        .blocks
        .iter()
        .find(|block| block.data == TEXT_BLOCK)
        .expect("WAX contains the game control labels");
    assert_eq!(text_block.load_address, 0x20000);

    let mut bus = Bus::new();
    connect_ram(&mut bus);
    bus.write_u8(0x10000, 0xAA); // This lies in the sparse hole before the string block.
    assert_eq!(load_bytes(&mut bus, TEST_WAX).unwrap(), 0);

    for (address, value) in [
        (0x20000, b'W'),
        (0x20011, b'J'),
        (0x20023, b'D'),
        (0x20026, b'S'),
        (0x20029, b'R'),
        (0x2002C, b'G'),
        (0x2002F, b'B'),
    ] {
        assert_eq!(bus.read_u8(address), value, "text byte at {address:#08X}");
    }
    assert_eq!(
        bus.read_u8(0x10000),
        0xAA,
        "sparse gap must remain untouched"
    );
}
