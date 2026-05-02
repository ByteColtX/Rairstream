use hkdf::{Hkdf, InvalidLength};
use sha2::Sha512;

pub fn derive_material(secret: &[u8], salt: &[u8], info: &[u8], output_len: usize) -> Vec<u8> {
    let hkdf = Hkdf::<Sha512>::new(Some(salt), secret);
    let mut output = vec![0_u8; output_len];
    if hkdf.expand(info, &mut output).is_err() {
        output.clear();
    }
    output
}

pub fn derive_fixed_32(secret: &[u8], salt: &[u8], info: &[u8]) -> Result<[u8; 32], InvalidLength> {
    let hkdf = Hkdf::<Sha512>::new(Some(salt), secret);
    let mut output = [0_u8; 32];
    hkdf.expand(info, &mut output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{derive_fixed_32, derive_material};

    #[test]
    fn derive_material_returns_requested_length() {
        let output = derive_material(b"secret", b"salt", b"info", 42);

        assert_eq!(output.len(), 42);
    }

    #[test]
    fn derive_fixed_32_returns_fixed_output() {
        let output = derive_fixed_32(b"secret", b"salt", b"info").unwrap();

        assert_eq!(output.len(), 32);
        assert_ne!(output, [0_u8; 32]);
    }
}
