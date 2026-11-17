//! Minimal HID report-descriptor parser.
//!
//! Only what is needed to locate battery fields: usages, report ids, sizes, counts and
//! logical ranges for Input and Feature main items. Collections are tracked only to
//! know the application usage page.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Input,
    Output,
    Feature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub kind: ReportKind,
    pub report_id: u8,
    /// Bit offset inside the report payload (after the report-id byte, if any).
    pub bit_offset: u32,
    pub bit_size: u32,
    pub usage_page: u16,
    pub usage: u16,
    pub logical_min: i32,
    pub logical_max: i32,
    pub constant: bool,
}

impl Field {
    /// Extract this field's raw value from a report buffer that starts with the report id
    /// when `report_id != 0`, or with payload data when it is 0.
    pub fn extract(&self, report: &[u8]) -> Option<i64> {
        let payload = if self.report_id != 0 { report.get(1..)? } else { report };
        let mut value: u64 = 0;
        for i in 0..self.bit_size {
            let bit = self.bit_offset + i;
            let byte = *payload.get((bit / 8) as usize)?;
            if byte >> (bit % 8) & 1 == 1 {
                value |= 1 << i;
            }
        }
        if self.logical_min < 0 && self.bit_size < 64 && value >> (self.bit_size - 1) & 1 == 1 {
            Some(value as i64 - (1i64 << self.bit_size))
        } else {
            Some(value as i64)
        }
    }

    /// Scale a raw value to 0..=100 using the logical range.
    pub fn to_percent(&self, raw: i64) -> u8 {
        let (min, max) = (self.logical_min as i64, self.logical_max as i64);
        if max <= min {
            return raw.clamp(0, 100) as u8;
        }
        (((raw - min) * 100 + (max - min) / 2) / (max - min)).clamp(0, 100) as u8
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Descriptor {
    pub fields: Vec<Field>,
    /// Usage page of the first top-level (application) collection.
    pub application_page: Option<u16>,
    /// Byte length (including id byte) of each report, keyed by (kind, id).
    pub report_lengths: Vec<(ReportKind, u8, usize)>,
}

impl Descriptor {
    pub fn report_len(&self, kind: ReportKind, id: u8) -> Option<usize> {
        self.report_lengths.iter().find(|(k, i, _)| *k == kind && *i == id).map(|(_, _, l)| *l)
    }
}

#[derive(Clone, Copy, Default)]
struct Globals {
    usage_page: u16,
    logical_min: i32,
    logical_max: i32,
    logical_max_raw: u32,
    report_size: u32,
    report_count: u32,
    report_id: u8,
}

fn signed(data: u32, size: usize) -> i32 {
    match size {
        1 => data as u8 as i8 as i32,
        2 => data as u16 as i16 as i32,
        _ => data as i32,
    }
}

pub fn parse(desc: &[u8]) -> Descriptor {
    let mut out = Descriptor::default();
    let mut g = Globals::default();
    let mut stack: Vec<Globals> = Vec::new();
    // Local usages: (page-or-0, usage). A page of 0 means "use the current global page".
    let mut usages: Vec<(u16, u16)> = Vec::new();
    let mut usage_min: Option<u32> = None;
    let mut depth = 0u32;
    // Running bit offset per (kind, report id).
    let mut offsets: Vec<(ReportKind, u8, u32)> = Vec::new();

    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        if prefix == 0xFE {
            // Long item: [FE][size][tag][data...]
            let size = *desc.get(i + 1).unwrap_or(&0) as usize;
            i += 3 + size;
            continue;
        }
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        let kind = (prefix >> 2) & 0x03;
        let tag = prefix >> 4;
        let mut data: u32 = 0;
        for b in 0..size {
            data |= (*desc.get(i + 1 + b).unwrap_or(&0) as u32) << (8 * b);
        }
        i += 1 + size;

        match (kind, tag) {
            // ---- Main items
            (0, 0x8) | (0, 0x9) | (0, 0xB) => {
                let rk = match tag {
                    0x8 => ReportKind::Input,
                    0x9 => ReportKind::Output,
                    _ => ReportKind::Feature,
                };
                let constant = data & 1 == 1;
                let entry = match offsets.iter_mut().find(|(k, id, _)| *k == rk && *id == g.report_id) {
                    Some(e) => e,
                    None => {
                        offsets.push((rk, g.report_id, 0));
                        offsets.last_mut().unwrap()
                    }
                };
                // Expand usage ranges lazily.
                let resolve = |n: usize| -> Option<(u16, u16)> {
                    if usages.is_empty() {
                        return None;
                    }
                    let (p, u) = usages[n.min(usages.len() - 1)];
                    Some((if p == 0 { g.usage_page } else { p }, u))
                };
                let mut lmax = g.logical_max;
                if lmax < g.logical_min {
                    // Descriptor declared an unsigned maximum that does not fit the signed encoding.
                    lmax = g.logical_max_raw.min(i32::MAX as u32) as i32;
                }
                for n in 0..g.report_count {
                    if let Some((page, usage)) = resolve(n as usize) {
                        out.fields.push(Field {
                            kind: rk,
                            report_id: g.report_id,
                            bit_offset: entry.2,
                            bit_size: g.report_size,
                            usage_page: page,
                            usage,
                            logical_min: g.logical_min,
                            logical_max: lmax,
                            constant,
                        });
                    }
                    entry.2 += g.report_size;
                }
                usages.clear();
                usage_min = None;
            }
            (0, 0xA) => {
                if depth == 0 && out.application_page.is_none() {
                    out.application_page =
                        Some(usages.first().map(|(p, _)| if *p == 0 { g.usage_page } else { *p }).unwrap_or(g.usage_page));
                }
                depth += 1;
                usages.clear();
                usage_min = None;
            }
            (0, 0xC) => depth = depth.saturating_sub(1),
            // ---- Global items
            (1, 0x0) => g.usage_page = data as u16,
            (1, 0x1) => g.logical_min = signed(data, size),
            (1, 0x2) => {
                g.logical_max = signed(data, size);
                g.logical_max_raw = data;
            }
            (1, 0x7) => g.report_size = data,
            (1, 0x8) => g.report_id = data as u8,
            (1, 0x9) => g.report_count = data,
            (1, 0xA) => stack.push(g),
            (1, 0xB) => g = stack.pop().unwrap_or(g),
            // ---- Local items
            (2, 0x0) => {
                if size == 4 {
                    usages.push(((data >> 16) as u16, data as u16));
                } else {
                    usages.push((0, data as u16));
                }
            }
            (2, 0x1) => usage_min = Some(data),
            (2, 0x2) => {
                if let Some(min) = usage_min.take() {
                    let page = if size == 4 { (data >> 16) as u16 } else { 0 };
                    let (lo, hi) = (min as u16, data as u16);
                    // Cap pathological ranges (e.g. keyboard 0..=0xFF) – battery usages are never in them.
                    for u in lo..=hi.min(lo.saturating_add(255)) {
                        usages.push((page, u));
                    }
                }
            }
            _ => {}
        }
    }

    for (k, id, bits) in offsets {
        let bytes = (bits as usize).div_ceil(8) + usize::from(id != 0);
        out.report_lengths.push((k, id, bytes));
    }
    out
}

/// Usages that carry a battery percentage.
pub const BATTERY_USAGES: [(u16, u16); 3] = [
    (0x06, 0x20), // Generic Device Controls: Battery Strength
    (0x85, 0x66), // Battery System: Relative State Of Charge
    (0x85, 0x65), // Battery System: Absolute State Of Charge
];

/// Usages that flag charging.
pub const CHARGING_USAGES: [(u16, u16); 2] = [
    (0x85, 0x44), // Charging
    (0x85, 0xD0), // AC Present
];

#[cfg(test)]
mod tests {
    use super::*;

    // Mouse with a feature report 0x05 carrying Battery Strength (0..=100) and an input report.
    const DESC: &[u8] = &[
        0x05, 0x01, 0x09, 0x02, 0xA1, 0x01, // Generic Desktop / Mouse / Application
        0x85, 0x01, 0x05, 0x09, 0x19, 0x01, 0x29, 0x03, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x03, 0x81, 0x02, // 3 buttons
        0x75, 0x05, 0x95, 0x01, 0x81, 0x03, // padding
        0x85, 0x05, 0x05, 0x06, 0x09, 0x20, 0x15, 0x00, 0x25, 0x64, 0x75, 0x08, 0x95, 0x01, 0xB1, 0x02, // Feature: Battery Strength
        0xC0,
    ];

    #[test]
    fn finds_battery_field() {
        let d = parse(DESC);
        assert_eq!(d.application_page, Some(0x01));
        let f = d.fields.iter().find(|f| (f.usage_page, f.usage) == (0x06, 0x20)).unwrap();
        assert_eq!(f.kind, ReportKind::Feature);
        assert_eq!(f.report_id, 5);
        assert_eq!(f.bit_offset, 0);
        assert_eq!(f.bit_size, 8);
        assert_eq!(f.logical_max, 100);
        assert_eq!(d.report_len(ReportKind::Feature, 5), Some(2));
        assert_eq!(d.report_len(ReportKind::Input, 1), Some(2));
        assert_eq!(f.extract(&[0x05, 87]), Some(87));
        assert_eq!(f.to_percent(87), 87);
    }

    #[test]
    fn scales_ranges_and_reads_unaligned_bits() {
        let f = Field {
            kind: ReportKind::Input,
            report_id: 2,
            bit_offset: 4,
            bit_size: 8,
            usage_page: 6,
            usage: 0x20,
            logical_min: 0,
            logical_max: 255,
            constant: false,
        };
        // payload bits 4..12 = 0xFF
        assert_eq!(f.extract(&[0x02, 0xF0, 0x0F]), Some(255));
        assert_eq!(f.to_percent(255), 100);
        assert_eq!(f.to_percent(128), 50);
    }
}
