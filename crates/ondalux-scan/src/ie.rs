//! IEEE 802.11 information element parsing (SSID and security), shared by
//! the Windows and Linux backends, which both expose raw beacon IEs.

use ondalux_core::survey::Security;

const EID_SSID: u8 = 0;
const EID_RSN: u8 = 48;
const EID_VENDOR: u8 = 221;
/// Microsoft OUI + type 1 = WPA (version 1) vendor element.
const WPA_OUI_TYPE: [u8; 4] = [0x00, 0x50, 0xF2, 0x01];
const IEEE_OUI: [u8; 3] = [0x00, 0x0F, 0xAC];
/// Privacy bit of the beacon capability information field.
pub const CAPABILITY_PRIVACY: u16 = 0x0010;

/// Iterates `(element id, body)` pairs; stops at the first truncated element.
pub fn elements(ies: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut rest = ies;
    std::iter::from_fn(move || {
        if rest.len() < 2 {
            return None;
        }
        let (id, len) = (rest[0], rest[1] as usize);
        if rest.len() < 2 + len {
            return None;
        }
        let body = &rest[2..2 + len];
        rest = &rest[2 + len..];
        Some((id, body))
    })
}

/// SSID from the IEs (lossy UTF-8; empty for hidden networks).
pub fn ssid(ies: &[u8]) -> Option<String> {
    elements(ies)
        .find(|(id, _)| *id == EID_SSID)
        .map(|(_, b)| String::from_utf8_lossy(b).trim_end_matches('\0').to_owned())
}

/// AKM suite types (IEEE 802.11 Table 9-151) from an RSN or WPA body.
/// `body` starts with the 2-byte version.
fn akm_suites(body: &[u8], oui: [u8; 3]) -> Vec<u8> {
    // version (2) + group cipher (4)
    let mut p = 6;
    let read_count = |p: usize| -> Option<usize> {
        body.get(p..p + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
    };
    let Some(pairwise) = read_count(p) else {
        return Vec::new();
    };
    p += 2 + 4 * pairwise;
    let Some(akms) = read_count(p) else {
        return Vec::new();
    };
    p += 2;
    (0..akms)
        .filter_map(|i| body.get(p + 4 * i..p + 4 * i + 4))
        .filter(|s| s[..3] == oui)
        .map(|s| s[3])
        .collect()
}

/// Security advertised by a BSS.
pub fn security(ies: &[u8], capability: u16) -> Security {
    let mut rsn_akms = None;
    let mut wpa_akms = None;
    for (id, body) in elements(ies) {
        match id {
            EID_RSN => rsn_akms = Some(akm_suites(body, IEEE_OUI)),
            EID_VENDOR if body.len() >= 4 && body[..4] == WPA_OUI_TYPE => {
                wpa_akms = Some(akm_suites(&body[4..], [0x00, 0x50, 0xF2]));
            }
            _ => {}
        }
    }
    if let Some(akms) = rsn_akms {
        let has = |set: &[u8]| akms.iter().any(|a| set.contains(a));
        // 802.1X variants: 1, 3 (FT), 5 (SHA-256), 11/12/13 (Suite B / FT-384).
        if has(&[1, 3, 5, 11, 12, 13]) {
            return Security::Enterprise;
        }
        // PSK variants: 2, 4 (FT), 6 (SHA-256). WPA3 transition mode
        // (PSK + SAE) is reported as WPA2 since WPA2 clients can join.
        if has(&[2, 4, 6]) {
            return Security::Wpa2Personal;
        }
        // SAE variants: 8, 9 (FT), 24, 25 (SAE-EXT-KEY).
        if has(&[8, 9, 24, 25]) {
            return Security::Wpa3Personal;
        }
        if has(&[18]) {
            return Security::Owe;
        }
        return Security::Unknown;
    }
    if let Some(akms) = wpa_akms {
        return if akms.contains(&1) {
            Security::Enterprise
        } else {
            Security::WpaPersonal
        };
    }
    if capability & CAPABILITY_PRIVACY != 0 {
        Security::Wep
    } else {
        Security::Open
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn rsn(akms: &[u8]) -> Vec<u8> {
        let mut b = vec![1, 0, 0x00, 0x0F, 0xAC, 4, 1, 0, 0x00, 0x0F, 0xAC, 4];
        b.extend_from_slice(&(akms.len() as u16).to_le_bytes());
        for a in akms {
            b.extend_from_slice(&[0x00, 0x0F, 0xAC, *a]);
        }
        b.extend_from_slice(&[0, 0]); // RSN capabilities
        let mut ie = vec![EID_RSN, b.len() as u8];
        ie.extend(b);
        ie
    }

    pub(crate) fn ssid_ie(s: &str) -> Vec<u8> {
        let mut ie = vec![EID_SSID, s.len() as u8];
        ie.extend_from_slice(s.as_bytes());
        ie
    }

    #[test]
    fn parses_ssid() {
        let mut ies = ssid_ie("Oficina");
        ies.extend(rsn(&[2]));
        assert_eq!(ssid(&ies).as_deref(), Some("Oficina"));
        assert_eq!(ssid(&ssid_ie("")).as_deref(), Some(""));
        assert_eq!(ssid(&[]), None);
    }

    #[test]
    fn classifies_security() {
        assert_eq!(security(&rsn(&[2]), 0x11), Security::Wpa2Personal);
        assert_eq!(security(&rsn(&[8]), 0x11), Security::Wpa3Personal);
        assert_eq!(security(&rsn(&[2, 8]), 0x11), Security::Wpa2Personal);
        assert_eq!(security(&rsn(&[1]), 0x11), Security::Enterprise);
        assert_eq!(security(&rsn(&[18]), 0x11), Security::Owe);
        assert_eq!(security(&ssid_ie("x"), CAPABILITY_PRIVACY), Security::Wep);
        assert_eq!(security(&ssid_ie("x"), 0x01), Security::Open);
        let wpa1 = [
            EID_VENDOR, 22, 0x00, 0x50, 0xF2, 1, 1, 0, 0x00, 0x50, 0xF2, 2, 1, 0, 0x00, 0x50, 0xF2,
            2, 1, 0, 0x00, 0x50, 0xF2, 2,
        ];
        assert_eq!(security(&wpa1, 0x11), Security::WpaPersonal);
    }

    #[test]
    fn truncated_elements_are_ignored() {
        let ies = [EID_SSID, 10, b'a', b'b'];
        assert_eq!(elements(&ies).count(), 0);
        assert_eq!(security(&ies, 0), Security::Open);
        // A truncated RSN body yields no AKMs rather than panicking.
        assert_eq!(security(&[EID_RSN, 2, 1, 0], 0x10), Security::Unknown);
    }
}
