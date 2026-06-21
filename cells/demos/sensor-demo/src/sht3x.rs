/// Temperature + humidity reading from an SHT3x sensor.
pub struct Reading {
    /// Temperature in tenths of °C (e.g., 253 = 25.3 °C).
    pub temp_cx10: i32,
    /// Relative humidity in tenths of percent (e.g., 610 = 61.0 %).
    pub hum_px10: u32,
    /// True when no sensor responded and synthetic data was substituted.
    pub simulated: bool,
}

/// Decode a 6-byte SHT3x high-precision single-shot response.
///
/// Byte layout: [T_MSB, T_LSB, T_CRC, H_MSB, H_LSB, H_CRC].
/// CRC bytes are ignored — the I2C bus itself is the error boundary.
///
/// Returns `None` when the response looks like a bus error (MSBs == 0xFF,
/// which signals "no slave pulled SDA low during the data phase").
pub fn parse(buf: &[u8; 6]) -> Option<Reading> {
    if buf[0] == 0xFF && buf[1] == 0xFF {
        return None;
    }
    let raw_t = ((buf[0] as u32) << 8) | buf[1] as u32;
    let raw_h = ((buf[3] as u32) << 8) | buf[4] as u32;
    // T[°C]  = -45 + 175 × raw_T / 65535
    // RH[%]  = 100 × raw_H / 65535
    let temp_cx10 = -450i32 + (1750i64 * raw_t as i64 / 65535) as i32;
    let hum_px10  = (1000u64 * raw_h as u64 / 65535) as u32;
    Some(Reading { temp_cx10, hum_px10, simulated: false })
}

/// Synthetic fallback when no I2C slave is present.
///
/// Walks a plausible range so the demo shows animated output rather
/// than a static value.  `tick` increments once per poll cycle.
pub fn synthetic(tick: u32) -> Reading {
    Reading {
        temp_cx10: 240 + (tick % 50) as i32,  // 24.0 → 28.9 °C
        hum_px10:  590 + (tick % 50) as u32,  // 59.0 → 63.9 %
        simulated: true,
    }
}
