//! Decoding for the link diagnostics (#17): the module EEPROM that MAD_IFC
//! reads (docs/spec/connectx3.md 5.11) and the PTYS register that ACCESS_REG
//! reads (5.12). Section numbers in comments are that document's.
//!
//! No UEFI and no allocation here, so `test/module.rs` can run it on the
//! host. Output goes to any `core::fmt::Write`, one line per `\n`.

use core::fmt::{self, Write};

/// MAD_IFC attribute ID of the vendor "module info" attribute (5.11.2).
pub const ATTR_MODULE_INFO: u16 = 0xff60;
/// The module's main I2C address, 7-bit form (5.11.2, 5.11.4).
pub const I2C_LOW: u8 = 0x50;
/// At most this many bytes per MAD_IFC request (5.11.4 item 4).
pub const MAX_READ: usize = 48;
/// PTYS register ID and size in bytes (5.12.1, 5.12.2).
pub const REG_PTYS: u16 = 0x5004;
pub const PTYS_LEN: usize = 0x34;
/// PTYS protocol mask: Ethernet (5.12.2).
pub const PROTO_ETH: u8 = 0x04;

/// The cable-info error in bits 15:8 of the MAD status (5.11.3).
pub fn mad_error(status: u16) -> &'static str {
    match status >> 8 {
        0x01 => "invalid port",
        0x02 => "operation not supported for this port",
        0x03 => "cable not connected",
        0x04 => "the cable has no EEPROM (passive copper cable)",
        0x05 => "page number greater than 15",
        0x06 => "invalid device address or size",
        0x07 => "invalid I2C device address",
        0x08 => "cable violates the QSFP specification (ignores ModSel)",
        0x09 => "I2C bus constantly busy",
        _ => "unknown cable-info error",
    }
}

/// Module kind from the identifier byte, or `None` when Linux would refuse
/// to read further (5.11.4 item 1).
pub fn kind(id: u8) -> Option<&'static str> {
    match id {
        0x03 => Some("SFP/SFP+"),
        0x0c => Some("QSFP"),
        0x0d => Some("QSFP+"),
        0x11 => Some("QSFP28"),
        _ => None,
    }
}

/// The I2C 0x50 page 0 byte ranges (start, length) the diagnostics need
/// (5.11.4): SFP bytes 0–95; QSFP bytes 0–2 and 128–223.
pub fn ranges(id: u8) -> &'static [(usize, usize)] {
    match id {
        0x03 => &[(0, 96)],
        0x0c | 0x0d | 0x11 => &[(0, 3), (128, 96)],
        _ => &[],
    }
}

/// Split one range into requests of at most `MAX_READ` bytes that never
/// pass device address 255 (5.11.4 items 4–5): (device address, size).
pub fn chunks(start: usize, len: usize) -> impl Iterator<Item = (usize, usize)> {
    let end = (start + len).min(256);
    (start..end).step_by(MAX_READ).map(move |a| (a, MAX_READ.min(end - a)))
}

/// ASCII field, trailing spaces and NULs dropped, anything else unprintable
/// shown as '.'.
struct Ascii<'a>(&'a [u8]);

impl fmt::Display for Ascii<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0.iter().rposition(|&b| b != b' ' && b != 0).map_or(0, |i| i + 1);
        for &b in &self.0[..n] {
            f.write_char(if (0x20..0x7f).contains(&b) { b as char } else { '.' })?;
        }
        Ok(())
    }
}

fn connector(b: u8) -> &'static str {
    match b {
        0x07 => "LC",
        0x21 => "copper pigtail",
        0x23 => "no separable connector",
        _ => "other",
    }
}

/// Raw bytes, 16 to a line, so the labels can be checked against the
/// cable's datasheet (spec 7 item 17).
pub fn dump<W: Write>(w: &mut W, e: &[u8; 256], start: usize, len: usize) -> fmt::Result {
    for line in (start..start + len).step_by(16) {
        write!(w, "eeprom {line:02x}:")?;
        for b in &e[line..(line + 16).min(start + len)] {
            write!(w, " {b:02x}")?;
        }
        writeln!(w)?;
    }
    Ok(())
}

/// Label what was read with the SFF fields of 5.11.5 (public SFF
/// standards, not the mlx4 sources; the raw bytes go alongside).
pub fn describe<W: Write>(w: &mut W, e: &[u8; 256]) -> fmt::Result {
    let id = e[0];
    match id {
        0x03 => {
            let cable = match e[8] & 0x0c {
                0x04 => "passive copper cable",
                0x08 => "active cable",
                0x0c => "passive and active bits both set",
                _ => "not a cable (no SFP+ cable technology bit)",
            };
            writeln!(w, "SFP/SFP+: {cable} (byte 8 {:#04x}), connector {:#04x} ({})", e[8], e[2], connector(e[2]))?;
            writeln!(
                w,
                "length {} m (byte 18), nominal rate {} MBd (byte 12), 10G compliance {:#03x} (byte 3 7:4), Ethernet compliance {:#04x} (byte 6)",
                e[18],
                u32::from(e[12]) * 100,
                e[3] >> 4,
                e[6]
            )?;
            writeln!(
                w,
                "vendor \"{}\" OUI {:02x}:{:02x}:{:02x}, part \"{}\" rev \"{}\", serial \"{}\"",
                Ascii(&e[20..36]),
                e[37],
                e[38],
                e[39],
                Ascii(&e[40..56]),
                Ascii(&e[56..60]),
                Ascii(&e[68..84])
            )?;
            writeln!(
                w,
                "cable compliance {:02x} {:02x} (bytes 60-61), diagnostics {} (byte 92 {:#04x})",
                e[60],
                e[61],
                if e[92] & 0x40 != 0 { "implemented" } else { "not implemented" },
                e[92]
            )
        }
        0x0c | 0x0d | 0x11 => {
            let tech = e[147] >> 4;
            let cable = match tech {
                0xa => "copper, unequalized",
                0xb => "passive copper, equalized",
                0xc..=0xf => "copper with active equalizers",
                _ => "optical transmitter",
            };
            writeln!(
                w,
                "{}: {cable} (byte 147 {:#04x}), connector {:#04x} ({}), revision {:#04x}, status {:#04x}",
                kind(id).unwrap_or("QSFP"),
                e[147],
                e[130],
                connector(e[130]),
                e[1],
                e[2]
            )?;
            let cr4 = if e[131] & 0x08 != 0 { ", 40GBASE-CR4" } else { "" };
            writeln!(w, "length {} m (byte 146), 10/40G compliance {:#04x}{cr4} (byte 131)", e[146], e[131])?;
            writeln!(
                w,
                "vendor \"{}\" OUI {:02x}:{:02x}:{:02x}, part \"{}\" rev \"{}\", serial \"{}\"",
                Ascii(&e[148..164]),
                e[165],
                e[166],
                e[167],
                Ascii(&e[168..184]),
                Ascii(&e[184..186]),
                Ascii(&e[196..212])
            )
        }
        _ => writeln!(w, "identifier {id:#04x}: not SFP or QSFP; not decoded"),
    }
}

/// Link-mode names by PTYS bit (5.12.3).
fn mode(bit: u32) -> Option<&'static str> {
    Some(match bit {
        0 => "1000BASE-CX-SGMII",
        1 => "1000BASE-KX",
        2 => "10GBASE-CX4",
        3 => "10GBASE-KX4",
        4 => "10GBASE-KR",
        5 => "20GBASE-KR2",
        6 => "40GBASE-CR4",
        7 => "40GBASE-KR4",
        8 => "56GBASE-KR4",
        12 => "10GBASE-CR",
        13 => "10GBASE-SR",
        15 => "40GBASE-SR4",
        17 => "56GBASE-CR4",
        18 => "56GBASE-SR4",
        24 => "100BASE-TX",
        25 => "1000BASE-T",
        26 => "10GBASE-T",
        _ => return None,
    })
}

/// A link-mode mask: the hex value, then each set bit's name; bits the
/// sources do not define are printed by number (5.12.3).
pub struct Modes(pub u32);

impl fmt::Display for Modes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#010x}", self.0)?;
        if self.0 == 0 {
            return f.write_str(" (none)");
        }
        let mut sep = " (";
        for bit in (0..32).filter(|b| self.0 & (1 << b) != 0) {
            f.write_str(sep)?;
            match mode(bit) {
                Some(m) => f.write_str(m)?,
                None => write!(f, "bit {bit}")?,
            }
            sep = ", ";
        }
        f.write_str(")")
    }
}

/// The PTYS fields of 5.12.2, from the register data (mailbox 0x14 on).
pub fn describe_ptys<W: Write>(w: &mut W, r: &[u8; PTYS_LEN]) -> fmt::Result {
    let be32 = |o: usize| u32::from_be_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]]);
    writeln!(
        w,
        "PTYS: local port {}, protocol {:#04x}, AN_DISABLE_CAP {}, AN_DISABLE_ADMIN {}",
        r[1],
        r[3],
        (r[0] >> 5) & 1,
        (r[0] >> 6) & 1
    )?;
    writeln!(w, "supported   {}", Modes(be32(0x0c)))?;
    writeln!(w, "advertised  {}", Modes(be32(0x18)))?;
    writeln!(w, "operating   {}", Modes(be32(0x24)))?;
    writeln!(w, "partner     {}", Modes(be32(0x30)))
}
