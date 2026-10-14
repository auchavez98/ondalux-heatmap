//! Access point list as CSV: one row per radio.

use crate::error::IoError;
use ondalux_core::ap::{AccessPoint, Mount};
use std::io::Write;

pub const HEADER: [&str; 17] = [
    "ap_id",
    "name",
    "vendor",
    "model",
    "x_m",
    "y_m",
    "height_m",
    "mount",
    "orientation_deg",
    "radio",
    "band",
    "channel",
    "width_mhz",
    "tx_power_dbm",
    "antenna_gain_dbi",
    "eirp_dbm",
    "enabled",
];

pub fn write_ap_csv<W: Write>(writer: W, aps: &[AccessPoint]) -> Result<(), IoError> {
    let mut w = csv::Writer::from_writer(writer);
    let err = |e: csv::Error| IoError::Format(format!("CSV: {e}"));
    w.write_record(HEADER).map_err(err)?;
    for ap in aps {
        for (i, r) in ap.radios.iter().enumerate() {
            w.write_record([
                ap.id.0.to_string(),
                ap.name.clone(),
                ap.vendor.clone(),
                ap.model.clone(),
                format!("{:.2}", ap.position.x),
                format!("{:.2}", ap.position.y),
                format!("{:.2}", ap.height_m),
                match ap.mount {
                    Mount::Ceiling => "ceiling".to_owned(),
                    Mount::Wall => "wall".to_owned(),
                },
                match ap.mount {
                    Mount::Ceiling => String::new(),
                    Mount::Wall => format!("{:.0}", ap.orientation_deg),
                },
                (i + 1).to_string(),
                r.channel.band.label().to_owned(),
                r.channel.number.to_string(),
                r.channel.width.mhz().to_string(),
                format!("{:.1}", r.tx_power.0),
                format!("{:.1}", r.antenna_gain.0),
                format!("{:.1}", r.eirp().0),
                r.enabled.to_string(),
            ])
            .map_err(err)?;
        }
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ondalux_core::ap::{ApId, Radio};
    use ondalux_core::channel::{Band, Channel, ChannelWidth};
    use ondalux_core::geometry::Vec2;
    use ondalux_core::units::{Db, Dbm};

    #[test]
    fn one_row_per_radio_with_eirp() {
        let ap = AccessPoint {
            id: ApId(3),
            name: "Lobby, east".into(),
            vendor: "Ubiquiti".into(),
            model: "U6 Pro".into(),
            position: Vec2::new(1.5, 2.25),
            height_m: 3.0,
            orientation_deg: 0.0,
            mount: Default::default(),
            pattern: Default::default(),
            radios: vec![
                Radio {
                    channel: Channel::new(Band::G2_4, 6, ChannelWidth::Mhz20).unwrap(),
                    tx_power: Dbm(17.0),
                    antenna_gain: Db(4.0),
                    enabled: true,
                },
                Radio {
                    channel: Channel::new(Band::G5, 36, ChannelWidth::Mhz80).unwrap(),
                    tx_power: Dbm(20.0),
                    antenna_gain: Db(6.0),
                    enabled: false,
                },
            ],
        };
        let mut out = Vec::new();
        write_ap_csv(&mut out, &[ap]).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], HEADER.join(","));
        assert_eq!(
            lines[1],
            "3,\"Lobby, east\",Ubiquiti,U6 Pro,1.50,2.25,3.00,ceiling,,1,2.4 GHz,6,20,17.0,4.0,21.0,true"
        );
        assert!(lines[2].ends_with("5 GHz,36,80,20.0,6.0,26.0,false"));
    }
}
