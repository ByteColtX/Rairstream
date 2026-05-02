use std::collections::HashSet;

use crate::error::RairstreamError;

use super::Receiver;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorMatch {
    Exact,
    Partial,
}

fn normalize_selector(selector: &str) -> String {
    selector.trim().to_ascii_lowercase()
}

fn normalized_device_id(value: &str) -> Option<String> {
    let normalized: String = value
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|character| character.to_ascii_lowercase())
        .collect();

    if normalized.is_empty() {
        return None;
    }

    Some(normalized)
}

fn receiver_match_kind(receiver: &Receiver, selector: &str) -> Option<SelectorMatch> {
    let selector = normalize_selector(selector);
    if selector.is_empty() {
        return None;
    }

    let id = receiver.id.to_ascii_lowercase();
    let name = receiver.name.to_ascii_lowercase();
    let host = receiver.host.to_ascii_lowercase();
    let endpoint = receiver.endpoint().to_ascii_lowercase();
    let selector_id = normalized_device_id(&selector);
    let receiver_id = normalized_device_id(&receiver.id);

    if id == selector
        || host == selector
        || endpoint == selector
        || name == selector
        || selector_id
            .zip(receiver_id)
            .is_some_and(|(selector, id)| selector == id)
    {
        return Some(SelectorMatch::Exact);
    }

    name.contains(&selector).then_some(SelectorMatch::Partial)
}

pub fn resolve_receiver(
    receivers: &[Receiver],
    selector: &str,
) -> Result<Receiver, RairstreamError> {
    let mut exact_matches = Vec::new();
    let mut partial_matches = Vec::new();

    for receiver in receivers {
        match receiver_match_kind(receiver, selector) {
            Some(SelectorMatch::Exact) => exact_matches.push(receiver.clone()),
            Some(SelectorMatch::Partial) => partial_matches.push(receiver.clone()),
            None => {}
        }
    }

    let matches = if exact_matches.is_empty() {
        partial_matches
    } else {
        exact_matches
    };

    match matches.len() {
        0 => Err(RairstreamError::ReceiverNotFound {
            selector: selector.to_string(),
        }),
        1 => Ok(matches.into_iter().next().unwrap()),
        _ => Err(RairstreamError::AmbiguousReceiver {
            selector: selector.to_string(),
            matches: matches
                .iter()
                .map(|receiver| receiver.name.clone())
                .collect(),
        }),
    }
}

pub fn resolve_receivers(
    receivers: &[Receiver],
    selectors: &[String],
) -> Result<Vec<Receiver>, RairstreamError> {
    if selectors.is_empty() {
        return Err(RairstreamError::InvalidInput {
            message: String::from("at least one --device selector is required"),
        });
    }

    let mut resolved = Vec::with_capacity(selectors.len());
    let mut seen = HashSet::new();
    for selector in selectors {
        let receiver = resolve_receiver(receivers, selector)?;
        if seen.insert(receiver.id.clone()) {
            resolved.push(receiver);
        }
    }

    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };

    use super::{resolve_receiver, resolve_receivers};

    fn build_receiver(id: &str, name: &str, host: &str) -> Receiver {
        Receiver {
            id: id.to_string(),
            name: name.to_string(),
            host: host.to_string(),
            port: 7000,
            generation: AirPlayGeneration::AirPlay1,
            transport_profile: ReceiverKind::ClassicRaop,
            support_level: DeviceSupport::Supported,
            auth_method: AuthMethod::None,
            capabilities: ReceiverCapabilities::default(),
            ..Receiver::default()
        }
        .with_compat_fields()
    }

    #[test]
    fn resolve_receiver_matches_name_substring() {
        let receiver = resolve_receiver(
            &[build_receiver("one", "Living Room", "192.168.1.10")],
            "living",
        )
        .unwrap();

        assert_eq!(receiver.id, "one");
    }

    #[test]
    fn resolve_receivers_deduplicates_repeated_matches() {
        let receivers = [build_receiver("one", "Living Room", "192.168.1.10")];
        let resolved = resolve_receivers(
            &receivers,
            &[String::from("living"), String::from("192.168.1.10")],
        )
        .unwrap();

        assert_eq!(resolved.len(), 1);
    }

    #[test]
    fn resolve_receiver_prefers_exact_name_over_substring_match() {
        let receiver = resolve_receiver(
            &[
                build_receiver("one", "Living Room", "192.168.1.10"),
                build_receiver("two", "Living Room TV", "192.168.1.11"),
            ],
            "living room",
        )
        .unwrap();

        assert_eq!(receiver.id, "one");
    }

    #[test]
    fn resolve_receiver_matches_colon_delimited_device_id() {
        let receiver = resolve_receiver(
            &[build_receiver(
                "001122334455",
                "Living Room",
                "192.168.1.10",
            )],
            "00:11:22:33:44:55",
        )
        .unwrap();

        assert_eq!(receiver.id, "001122334455");
    }

    #[test]
    fn resolve_receiver_matches_endpoint() {
        let receiver = resolve_receiver(
            &[build_receiver("one", "Living Room", "192.168.1.10")],
            "192.168.1.10:7000",
        )
        .unwrap();

        assert_eq!(receiver.id, "one");
    }
}
