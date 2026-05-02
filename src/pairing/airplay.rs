use std::io::Cursor;

use aes::Aes128;
use aes::cipher::{KeyIvInit, StreamCipher};
use aes_gcm::aead::{AeadInPlace, KeyInit as AesGcmKeyInit};
use base64ct::{Base64, Base64Unpadded, Encoding};
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ctr::Ctr128BE;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use plist::{Dictionary, Value};
use rand_core::{OsRng, RngCore};
use sha1::{Digest as Sha1Digest, Sha1 as SrpSha1};
use sha2::Digest;
use sha2::Sha512 as HkdfSha512;
use srp::ClientG2048;
use srp::Group;
use srp::groups::G2048;
use srp::utils::compute_m1_rfc5054;
use tracing::{debug, info};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

use super::{ReceiverAuthFlow, ReceiverCredentials};
use crate::receiver::Receiver;
use crate::rtsp::{RtspClient, RtspResponse};
use crate::session::{AirPlayError, ModernAirPlayConnection, ModernAirPlaySession, RaopSession};

pub(crate) const PAIRING_TLV_IDENTIFIER: u8 = 0x01;
pub(crate) const PAIRING_TLV_PUBLIC_KEY: u8 = 0x03;
pub(crate) const PAIRING_TLV_ENCRYPTED_DATA: u8 = 0x05;
pub(crate) const PAIRING_TLV_STATE: u8 = 0x06;
const PAIRING_TLV_ERROR: u8 = 0x07;
pub(crate) const PAIRING_TLV_SIGNATURE: u8 = 0x0A;

const PAIR_VERIFY_START_REQUEST_STATE: u8 = 1;
const PAIR_VERIFY_START_RESPONSE_STATE: u8 = 2;
const PAIR_VERIFY_FINISH_REQUEST_STATE: u8 = 3;
pub(crate) const PAIR_VERIFY_FINISH_RESPONSE_STATE: u8 = 4;

const AUTH_SETUP_ENCRYPTION_TYPE_UNENCRYPTED: u8 = 0x01;
pub(crate) const PAIR_VERIFY_ENCRYPT_SALT: &[u8] = b"Pair-Verify-Encrypt-Salt";
pub(crate) const PAIR_VERIFY_ENCRYPT_INFO: &[u8] = b"Pair-Verify-Encrypt-Info";
pub(crate) const PAIR_VERIFY_M2_NONCE: &[u8] = b"PV-Msg02";
const PAIR_VERIFY_M3_NONCE: &[u8] = b"PV-Msg03";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairingTlvEntry {
    type_id: u8,
    value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PairingTlv {
    entries: Vec<PairingTlvEntry>,
}

impl PairingTlv {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&mut self, type_id: u8, value: impl Into<Vec<u8>>) {
        self.entries.push(PairingTlvEntry {
            type_id,
            value: value.into(),
        });
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::new();

        for entry in &self.entries {
            if entry.value.is_empty() {
                encoded.push(entry.type_id);
                encoded.push(0);
                continue;
            }

            for chunk in entry.value.chunks(usize::from(u8::MAX)) {
                encoded.push(entry.type_id);
                encoded.push(u8::try_from(chunk.len()).unwrap_or(u8::MAX));
                encoded.extend_from_slice(chunk);
            }
        }

        encoded
    }

    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, AirPlayError> {
        let mut entries: Vec<PairingTlvEntry> = Vec::new();
        let mut cursor = 0_usize;

        while cursor < bytes.len() {
            if cursor + 2 > bytes.len() {
                return Err(AirPlayError::Protocol {
                    message: String::from("pairing TLV is missing the type/length fields"),
                });
            }

            let type_id = bytes[cursor];
            let value_len = usize::from(bytes[cursor + 1]);
            cursor += 2;

            if cursor + value_len > bytes.len() {
                return Err(AirPlayError::Protocol {
                    message: String::from("pairing TLV length exceeds response body bounds"),
                });
            }

            let value = &bytes[cursor..cursor + value_len];
            cursor += value_len;

            if let Some(last_entry) = entries.last_mut()
                && last_entry.type_id == type_id
            {
                last_entry.value.extend_from_slice(value);
            } else {
                entries.push(PairingTlvEntry {
                    type_id,
                    value: value.to_vec(),
                });
            }
        }

        Ok(Self { entries })
    }

    pub(crate) fn get(&self, type_id: u8) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|entry| entry.type_id == type_id)
            .map(|entry| entry.value.as_slice())
    }

    pub(crate) fn require(&self, type_id: u8, field_name: &str) -> Result<&[u8], AirPlayError> {
        self.get(type_id).ok_or_else(|| AirPlayError::Protocol {
            message: format!("pairing TLV is missing field {field_name}"),
        })
    }

    pub(crate) fn require_byte(&self, type_id: u8, field_name: &str) -> Result<u8, AirPlayError> {
        let value = self.require(type_id, field_name)?;
        if value.len() != 1 {
            return Err(AirPlayError::Protocol {
                message: format!(
                    "pairing TLV field {field_name} must be 1 byte, got {}",
                    value.len()
                ),
            });
        }

        Ok(value[0])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PairVerifyStartReply {
    public_key: [u8; 32],
    encrypted_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyEncryptedPublicKey {
    ciphertext: Vec<u8>,
    tag: [u8; 16],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyPairSetupPinStartReply {
    salt: Vec<u8>,
    public_key: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyPairSetupPinVerifyReply {
    proof: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LegacyPairVerifyStartReply {
    public_key: [u8; 32],
    challenge: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModernAuthProbeOutcome {
    ContinueWithCredentials,
    RequiresPairing,
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedReceiverCredentials {
    pub(crate) auth_flow: ReceiverAuthFlow,
    pub(crate) controller_pairing_id: String,
    pub(crate) controller_signing_key: SigningKey,
    pub(crate) receiver_pairing_id: String,
    pub(crate) receiver_verifying_key: VerifyingKey,
}

impl ModernAirPlaySession {
    pub fn handshake(mut self) -> Result<ModernAirPlayConnection, AirPlayError> {
        let endpoint = self.descriptor().device.endpoint();
        info!(
            endpoint = %endpoint,
            device_id = %self.descriptor().device.id,
            "starting AirPlay Receiver authentication recovery"
        );
        let mut rtsp_client = RtspClient::connect(&endpoint)?;
        let info_response = rtsp_client.send(&self.info_request())?;
        debug!(
            endpoint = %endpoint,
            status_code = info_response.status.code,
            "AirPlay Receiver /info responded"
        );

        if self.descriptor().receiver_credentials.is_none() {
            return detect_initial_pairing_requirement(&mut rtsp_client, &mut self, &endpoint);
        }

        match map_modern_auth_probe_response(&info_response)? {
            ModernAuthProbeOutcome::RequiresPairing => Err(AirPlayError::PairingRequired),
            ModernAuthProbeOutcome::ContinueWithCredentials => {
                let receiver_credentials = self.receiver_credentials()?;
                match receiver_credentials.auth_flow {
                    ReceiverAuthFlow::Modern => {
                        complete_pair_verify(&mut rtsp_client, &mut self, &receiver_credentials)?;
                        complete_auth_setup(&mut rtsp_client, &mut self)?;
                        let raop_connection = RaopSession::connect(self.descriptor())?
                            .with_initial_cseq(self.cseq())
                            .handshake_with_rtsp_client(rtsp_client)?;
                        Ok(ModernAirPlayConnection::from_raop(raop_connection))
                    }
                    ReceiverAuthFlow::LegacyPin => {
                        complete_legacy_pair_verify(
                            &mut rtsp_client,
                            &mut self,
                            &receiver_credentials.controller_signing_key,
                            &receiver_credentials.receiver_verifying_key.to_bytes(),
                        )?;
                        complete_optional_auth_setup(&mut rtsp_client, &mut self)?;
                        let raop_connection = RaopSession::connect(self.descriptor())?
                            .with_initial_cseq(self.cseq())
                            .handshake_with_rtsp_client(rtsp_client)?;
                        Ok(ModernAirPlayConnection::from_raop(raop_connection))
                    }
                }
            }
        }
    }

    pub fn pair_with_pin(mut self, pin: &str) -> Result<ReceiverCredentials, AirPlayError> {
        let endpoint = self.descriptor().device.endpoint();
        info!(
            endpoint = %endpoint,
            device_id = %self.descriptor().device.id,
            "starting first-time AirPlay Receiver pairing"
        );
        let mut rtsp_client = RtspClient::connect(&endpoint)?;
        let _info_response = rtsp_client.send(&self.info_request())?;
        debug!(
            endpoint = %endpoint,
            "AirPlay Receiver /info responded before first-time pairing"
        );
        complete_legacy_pairing(&mut rtsp_client, &mut self, pin)
    }

    pub fn request_pairing_pin_display(mut self) -> Result<(), AirPlayError> {
        let endpoint = self.descriptor().device.endpoint();
        info!(
            endpoint = %endpoint,
            device_id = %self.descriptor().device.id,
            "requesting AirPlay Receiver pairing PIN display"
        );
        let mut rtsp_client = RtspClient::connect(&endpoint)?;
        let _info_response = rtsp_client.send(&self.info_request())?;
        debug!(
            endpoint = %endpoint,
            "AirPlay Receiver /info responded before PIN display"
        );
        let pair_pin_start_response = rtsp_client.send(&self.pair_pin_start_request())?;
        debug!(
            endpoint = %endpoint,
            status_code = pair_pin_start_response.status.code,
            "AirPlay Receiver /pair-pin-start request"
        );
        map_pair_pin_start_response(&pair_pin_start_response)
    }

    pub(crate) fn receiver_credentials(&self) -> Result<DecodedReceiverCredentials, AirPlayError> {
        let Some(receiver_credentials) = self.descriptor().receiver_credentials.as_ref() else {
            return Err(AirPlayError::CredentialsMissing);
        };

        decode_receiver_credentials(receiver_credentials, &self.descriptor().device)
    }
}

pub(crate) fn map_modern_auth_probe_response(
    response: &RtspResponse,
) -> Result<ModernAuthProbeOutcome, AirPlayError> {
    match response.status.code {
        200 => Ok(ModernAuthProbeOutcome::ContinueWithCredentials),
        401 => Ok(ModernAuthProbeOutcome::RequiresPairing),
        403 => Err(AirPlayError::AuthenticationFailed {
            message: String::from(
                "receiver rejected the current authentication context or anonymous control probe",
            ),
        }),
        code => Err(AirPlayError::Protocol {
            message: format!("AirPlay Receiver /info returned unexpected status {code}"),
        }),
    }
}

pub(crate) fn detect_initial_pairing_requirement(
    rtsp_client: &mut RtspClient,
    session: &mut ModernAirPlaySession,
    endpoint: &str,
) -> Result<ModernAirPlayConnection, AirPlayError> {
    let pair_pin_start_response = rtsp_client.send(&session.pair_pin_start_request())?;
    debug!(
        endpoint = %endpoint,
        status_code = pair_pin_start_response.status.code,
        "probing /pair-pin-start because no local AirPlay Receiver credentials are available"
    );
    map_pair_pin_start_response(&pair_pin_start_response)?;
    Err(AirPlayError::PairingRequired)
}

#[allow(deprecated)]
pub(crate) fn complete_legacy_pairing(
    rtsp_client: &mut RtspClient,
    session: &mut ModernAirPlaySession,
    pin: &str,
) -> Result<ReceiverCredentials, AirPlayError> {
    let trimmed_pin = pin.trim();
    if trimmed_pin.is_empty() {
        return Err(AirPlayError::AuthenticationFailed {
            message: String::from("pairing PIN must not be empty"),
        });
    }

    let pair_setup_user = session.descriptor().client_device_id();
    let pair_pin_start_response = rtsp_client.send(&session.pair_pin_start_request())?;
    map_pair_pin_start_response(&pair_pin_start_response)?;

    let start_response = rtsp_client.send(
        &session.pair_setup_pin_request(build_pair_setup_pin_start_body(&pair_setup_user)?),
    )?;
    let start_reply = map_pair_setup_pin_start_response(&start_response)?;

    let mut controller_secret = [0_u8; 48];
    OsRng.fill_bytes(&mut controller_secret);
    let srp_client = ClientG2048::<SrpSha1>::new();
    let controller_public = srp_client.compute_public_ephemeral(&controller_secret);
    let padded_controller_public = left_pad_legacy_srp_public(&controller_public)?;
    let verifier = srp_client
        .process_reply(
            &controller_secret,
            pair_setup_user.as_bytes(),
            trimmed_pin.as_bytes(),
            &start_reply.salt,
            &start_reply.public_key,
        )
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from(
                "pair-setup-pin SRP parameters are invalid, or the provided PIN is incorrect",
            ),
        })?;
    let session_key = derive_legacy_pair_setup_session_key(verifier.key());
    let padded_receiver_public = left_pad_legacy_srp_public(&start_reply.public_key)?;
    let client_proof = build_legacy_pair_setup_client_proof(
        &pair_setup_user,
        &start_reply.salt,
        &padded_controller_public,
        &padded_receiver_public,
        &session_key,
    );

    let verify_response = rtsp_client.send(&session.pair_setup_pin_request(
        build_pair_setup_pin_verify_body(&padded_controller_public, &client_proof)?,
    ))?;
    let verify_reply = map_pair_setup_pin_verify_response(&verify_response)?;
    verify_legacy_pair_setup_server_proof(
        &padded_controller_public,
        &client_proof,
        &session_key,
        &verify_reply.proof,
    )?;

    let mut controller_seed = [0_u8; 32];
    OsRng.fill_bytes(&mut controller_seed);
    let controller_signing_key = SigningKey::from_bytes(&controller_seed);
    let controller_public_key = controller_signing_key.verifying_key().to_bytes();
    let encrypted_public_key =
        encrypt_legacy_pair_setup_public_key(&session_key, &controller_public_key)?;
    let exchange_response = rtsp_client.send(&session.pair_setup_pin_request(
        build_pair_setup_pin_exchange_body(
            &encrypted_public_key.ciphertext,
            &encrypted_public_key.tag,
        )?,
    ))?;
    let receiver_public_key =
        map_pair_setup_pin_exchange_response(&exchange_response, &session_key)?;

    let receiver_pairing_id = complete_legacy_pair_verify(
        rtsp_client,
        session,
        &controller_signing_key,
        &receiver_public_key,
    )?;

    Ok(ReceiverCredentials {
        auth_flow: ReceiverAuthFlow::LegacyPin,
        controller_pairing_id: pair_setup_user.clone(),
        controller_ltpk_hex: hex::encode(controller_public_key),
        controller_ltsk_hex: hex::encode(controller_signing_key.to_bytes()),
        receiver_pairing_id,
        receiver_ltpk_hex: hex::encode(receiver_public_key),
    })
}

pub(crate) fn complete_pair_verify(
    rtsp_client: &mut RtspClient,
    session: &mut ModernAirPlaySession,
    receiver_credentials: &DecodedReceiverCredentials,
) -> Result<(), AirPlayError> {
    let controller_secret = EphemeralSecret::random_from_rng(OsRng);
    let controller_public = X25519PublicKey::from(&controller_secret);
    let start_request = build_pair_verify_start_body(controller_public.as_bytes());
    let start_response = rtsp_client.send(&session.pair_verify_request(start_request))?;
    let start_reply = map_pair_verify_start_response(&start_response)?;
    let shared_secret =
        controller_secret.diffie_hellman(&X25519PublicKey::from(start_reply.public_key));
    let verify_key = derive_hkdf_sha512(
        shared_secret.as_bytes(),
        PAIR_VERIFY_ENCRYPT_SALT,
        PAIR_VERIFY_ENCRYPT_INFO,
    )?;
    verify_receiver_identity(
        receiver_credentials,
        controller_public.as_bytes(),
        &start_reply,
        &verify_key,
    )?;
    let finish_request = build_pair_verify_finish_body(
        receiver_credentials,
        controller_public.as_bytes(),
        &start_reply.public_key,
        &verify_key,
    )?;
    let finish_response = rtsp_client.send(&session.pair_verify_request(finish_request))?;
    map_pair_verify_finish_response(&finish_response)?;
    Ok(())
}

pub(crate) fn complete_auth_setup(
    rtsp_client: &mut RtspClient,
    session: &mut ModernAirPlaySession,
) -> Result<(), AirPlayError> {
    let client_secret = EphemeralSecret::random_from_rng(OsRng);
    let client_public = X25519PublicKey::from(&client_secret);
    let response = rtsp_client
        .send(&session.auth_setup_request(build_auth_setup_body(client_public.as_bytes())))?;
    if !response.is_success() {
        return Err(map_pairing_http_failure(&response, "/auth-setup"));
    }
    Ok(())
}

pub(crate) fn complete_optional_auth_setup(
    rtsp_client: &mut RtspClient,
    session: &mut ModernAirPlaySession,
) -> Result<(), AirPlayError> {
    let client_secret = EphemeralSecret::random_from_rng(OsRng);
    let client_public = X25519PublicKey::from(&client_secret);
    let response = rtsp_client
        .send(&session.auth_setup_request(build_auth_setup_body(client_public.as_bytes())))?;
    if response.is_success() {
        return Ok(());
    }
    if response.status.code == 404 {
        debug!(
            "receiver did not expose /auth-setup; continuing with the remaining RAOP setup flow"
        );
        return Ok(());
    }
    Err(map_pairing_http_failure(&response, "/auth-setup"))
}

fn build_pair_verify_start_body(controller_public: &[u8; 32]) -> Vec<u8> {
    let mut tlv = PairingTlv::new();
    tlv.push(PAIRING_TLV_STATE, [PAIR_VERIFY_START_REQUEST_STATE]);
    tlv.push(PAIRING_TLV_PUBLIC_KEY, *controller_public);
    tlv.encode()
}

fn build_pair_verify_finish_body(
    receiver_credentials: &DecodedReceiverCredentials,
    controller_public: &[u8; 32],
    receiver_public: &[u8; 32],
    verify_key: &[u8; 32],
) -> Result<Vec<u8>, AirPlayError> {
    let mut plaintext = PairingTlv::new();
    plaintext.push(
        PAIRING_TLV_IDENTIFIER,
        receiver_credentials.controller_pairing_id.as_bytes(),
    );
    let mut signed_message =
        Vec::with_capacity(32 + receiver_credentials.controller_pairing_id.len() + 32);
    signed_message.extend_from_slice(controller_public);
    signed_message.extend_from_slice(receiver_credentials.controller_pairing_id.as_bytes());
    signed_message.extend_from_slice(receiver_public);
    let signature = receiver_credentials
        .controller_signing_key
        .sign(&signed_message);
    plaintext.push(PAIRING_TLV_SIGNATURE, signature.to_bytes());
    let encrypted_data =
        encrypt_pair_verify_payload(verify_key, PAIR_VERIFY_M3_NONCE, &plaintext.encode())?;

    let mut tlv = PairingTlv::new();
    tlv.push(PAIRING_TLV_STATE, [PAIR_VERIFY_FINISH_REQUEST_STATE]);
    tlv.push(PAIRING_TLV_ENCRYPTED_DATA, encrypted_data);
    Ok(tlv.encode())
}

fn build_auth_setup_body(client_public_key: &[u8; 32]) -> [u8; 33] {
    let mut body = [0_u8; 33];
    body[0] = AUTH_SETUP_ENCRYPTION_TYPE_UNENCRYPTED;
    body[1..].copy_from_slice(client_public_key);
    body
}

fn build_pair_setup_pin_start_body(controller_pairing_id: &str) -> Result<Vec<u8>, AirPlayError> {
    let mut dictionary = Dictionary::new();
    dictionary.insert(String::from("method"), Value::String(String::from("pin")));
    dictionary.insert(
        String::from("user"),
        Value::String(controller_pairing_id.to_string()),
    );
    encode_binary_plist(&Value::Dictionary(dictionary))
}

fn build_pair_setup_pin_verify_body(
    controller_public: &[u8],
    proof: &[u8],
) -> Result<Vec<u8>, AirPlayError> {
    let mut dictionary = Dictionary::new();
    dictionary.insert(String::from("pk"), Value::Data(controller_public.to_vec()));
    dictionary.insert(String::from("proof"), Value::Data(proof.to_vec()));
    encode_binary_plist(&Value::Dictionary(dictionary))
}

fn build_pair_setup_pin_exchange_body(
    encrypted_public_key: &[u8],
    auth_tag: &[u8],
) -> Result<Vec<u8>, AirPlayError> {
    let mut dictionary = Dictionary::new();
    dictionary.insert(
        String::from("epk"),
        Value::Data(encrypted_public_key.to_vec()),
    );
    dictionary.insert(String::from("authTag"), Value::Data(auth_tag.to_vec()));
    encode_binary_plist(&Value::Dictionary(dictionary))
}

fn build_legacy_pair_verify_start_body(
    controller_curve_public: &[u8; 32],
    controller_signing_public: &[u8; 32],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(68);
    body.extend_from_slice(&[1, 0, 0, 0]);
    body.extend_from_slice(controller_curve_public);
    body.extend_from_slice(controller_signing_public);
    body
}

fn build_legacy_pair_verify_finish_body(
    controller_curve_public: &[u8; 32],
    receiver_curve_public: &[u8; 32],
    challenge: &[u8; 64],
    controller_signing_key: &SigningKey,
    verify_key: &[u8; 16],
    verify_iv: &[u8; 16],
) -> Vec<u8> {
    let mut signed_message = Vec::with_capacity(64);
    signed_message.extend_from_slice(controller_curve_public);
    signed_message.extend_from_slice(receiver_curve_public);
    let signature = controller_signing_key.sign(&signed_message);
    let encrypted_signature = apply_legacy_pair_verify_finish_signature_keystream(
        verify_key,
        verify_iv,
        challenge,
        &signature.to_bytes(),
    );
    let mut body = Vec::with_capacity(68);
    body.extend_from_slice(&[0, 0, 0, 0]);
    body.extend_from_slice(&encrypted_signature);
    body
}

pub(crate) fn encode_binary_plist(value: &Value) -> Result<Vec<u8>, AirPlayError> {
    let mut encoded = Vec::new();
    value
        .to_writer_binary(&mut encoded)
        .map_err(|error| AirPlayError::Protocol {
            message: format!("failed to encode binary plist: {error}"),
        })?;
    Ok(encoded)
}

fn decode_binary_plist(response: &RtspResponse, path: &str) -> Result<Dictionary, AirPlayError> {
    let value = Value::from_reader(Cursor::new(&response.body)).map_err(|error| {
        AirPlayError::Protocol {
            message: format!("{path} returned an invalid binary plist: {error}"),
        }
    })?;
    value
        .into_dictionary()
        .ok_or_else(|| AirPlayError::Protocol {
            message: format!("{path} returned a binary plist that is not a dictionary"),
        })
}

pub(crate) fn derive_hkdf_sha512(
    input_key_material: &[u8],
    salt: &[u8],
    info: &[u8],
) -> Result<[u8; 32], AirPlayError> {
    let hkdf = Hkdf::<HkdfSha512>::new(Some(salt), input_key_material);
    let mut output = [0_u8; 32];
    hkdf.expand(info, &mut output)
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from("failed to derive the session authentication key"),
        })?;
    Ok(output)
}

pub(crate) fn map_pair_pin_start_response(response: &RtspResponse) -> Result<(), AirPlayError> {
    if response.is_success() {
        return Ok(());
    }

    Err(map_pairing_http_failure(response, "/pair-pin-start"))
}

fn map_pair_setup_pin_start_response(
    response: &RtspResponse,
) -> Result<LegacyPairSetupPinStartReply, AirPlayError> {
    if !response.is_success() {
        return Err(map_pairing_http_failure(response, "/pair-setup-pin"));
    }

    let dictionary = decode_binary_plist(response, "/pair-setup-pin")?;
    let salt = plist_data_field(&dictionary, "salt", "/pair-setup-pin")?;
    let public_key = plist_data_field(&dictionary, "pk", "/pair-setup-pin")?;
    Ok(LegacyPairSetupPinStartReply { salt, public_key })
}

fn map_pair_setup_pin_verify_response(
    response: &RtspResponse,
) -> Result<LegacyPairSetupPinVerifyReply, AirPlayError> {
    if !response.is_success() {
        return Err(map_pairing_http_failure(response, "/pair-setup-pin"));
    }

    let dictionary = decode_binary_plist(response, "/pair-setup-pin")?;
    let proof = plist_data_field(&dictionary, "proof", "/pair-setup-pin")?;
    Ok(LegacyPairSetupPinVerifyReply { proof })
}

fn map_pair_setup_pin_exchange_response(
    response: &RtspResponse,
    session_key: &[u8],
) -> Result<[u8; 32], AirPlayError> {
    if !response.is_success() {
        return Err(map_pairing_http_failure(response, "/pair-setup-pin"));
    }

    let dictionary = decode_binary_plist(response, "/pair-setup-pin")?;
    let encrypted_public_key = plist_data_field(&dictionary, "epk", "/pair-setup-pin")?;
    let auth_tag = plist_data_field(&dictionary, "authTag", "/pair-setup-pin")?;
    decrypt_legacy_pair_setup_public_key(session_key, &encrypted_public_key, &auth_tag)
}

pub(crate) fn complete_legacy_pair_verify(
    rtsp_client: &mut RtspClient,
    session: &mut ModernAirPlaySession,
    controller_signing_key: &SigningKey,
    receiver_public_key: &[u8; 32],
) -> Result<String, AirPlayError> {
    let receiver_pairing_id = receiver_pairing_id_from_device(&session.descriptor().device)?;
    let controller_secret = EphemeralSecret::random_from_rng(OsRng);
    let controller_public = X25519PublicKey::from(&controller_secret);
    let start_request = build_legacy_pair_verify_start_body(
        controller_public.as_bytes(),
        &controller_signing_key.verifying_key().to_bytes(),
    );
    let start_response = rtsp_client.send(&session.pair_verify_request(start_request))?;
    let start_reply = map_legacy_pair_verify_start_response(&start_response)?;
    let shared_secret =
        controller_secret.diffie_hellman(&X25519PublicKey::from(start_reply.public_key));
    let (verify_key, verify_iv) = derive_legacy_pair_verify_key_iv(shared_secret.as_bytes());
    verify_legacy_pair_verify_signature(
        &start_reply.public_key,
        controller_public.as_bytes(),
        &start_reply.challenge,
        receiver_public_key,
        &verify_key,
        &verify_iv,
    )?;
    let finish_request = build_legacy_pair_verify_finish_body(
        controller_public.as_bytes(),
        &start_reply.public_key,
        &start_reply.challenge,
        controller_signing_key,
        &verify_key,
        &verify_iv,
    );
    let finish_response = rtsp_client.send(&session.pair_verify_request(finish_request))?;
    if !finish_response.is_success() {
        return Err(map_pairing_http_failure(&finish_response, "/pair-verify"));
    }
    Ok(receiver_pairing_id)
}

fn receiver_pairing_id_from_device(device: &Receiver) -> Result<String, AirPlayError> {
    device
        .pairing_id
        .clone()
        .ok_or_else(|| AirPlayError::InvalidSession {
            message: String::from(
                "receiver is missing an AirPlay Receiver pairing id (mDNS pi/gid)",
            ),
        })
}

fn decode_mdns_public_key(value: &str) -> Result<[u8; 32], AirPlayError> {
    if let Ok(bytes) = hex::decode(value) {
        return bytes.try_into().map_err(|_| AirPlayError::InvalidSession {
            message: String::from("mDNS pk requires 32 bytes of hex data"),
        });
    }

    let bytes = Base64::decode_vec(value)
        .or_else(|_| Base64Unpadded::decode_vec(value))
        .map_err(|_| AirPlayError::InvalidSession {
            message: String::from("mDNS pk is not a valid hex or Base64 public key"),
        })?;
    bytes.try_into().map_err(|_| AirPlayError::InvalidSession {
        message: String::from("mDNS pk requires 32 bytes of public key data"),
    })
}

fn plist_data_field(
    dictionary: &Dictionary,
    field_name: &str,
    path: &str,
) -> Result<Vec<u8>, AirPlayError> {
    match dictionary.get(field_name) {
        Some(Value::Data(value)) => Ok(value.clone()),
        Some(Value::String(value)) => Base64::decode_vec(value)
            .or_else(|_| Base64Unpadded::decode_vec(value))
            .map_err(|_| AirPlayError::Protocol {
                message: format!("{path} field {field_name} is not valid Base64 data"),
            }),
        Some(_) => Err(AirPlayError::Protocol {
            message: format!("{path} field {field_name} has an invalid type"),
        }),
        None => Err(AirPlayError::Protocol {
            message: format!("{path} is missing field {field_name}"),
        }),
    }
}

pub(crate) fn derive_legacy_pair_setup_session_key(shared_secret: &[u8]) -> Vec<u8> {
    let mut first_hasher = SrpSha1::new();
    first_hasher.update(shared_secret);
    first_hasher.update([0_u8, 0, 0, 0]);
    let first = first_hasher.finalize();

    let mut second_hasher = SrpSha1::new();
    second_hasher.update(shared_secret);
    second_hasher.update([0_u8, 0, 0, 1]);
    let second = second_hasher.finalize();

    let mut session_key = Vec::with_capacity(first.len() + second.len());
    session_key.extend_from_slice(&first);
    session_key.extend_from_slice(&second);
    session_key
}

pub(crate) fn derive_legacy_pair_setup_key_iv(session_key: &[u8]) -> ([u8; 16], [u8; 16]) {
    let mut key_hasher = HkdfSha512::new();
    key_hasher.update(b"Pair-Setup-AES-Key");
    key_hasher.update(session_key);
    let key_digest = key_hasher.finalize();

    let mut iv_hasher = HkdfSha512::new();
    iv_hasher.update(b"Pair-Setup-AES-IV");
    iv_hasher.update(session_key);
    let iv_digest = iv_hasher.finalize();

    let mut key = [0_u8; 16];
    key.copy_from_slice(&key_digest[..16]);
    let mut iv = [0_u8; 16];
    iv.copy_from_slice(&iv_digest[..16]);
    increment_big_endian_counter(&mut iv);
    (key, iv)
}

pub(crate) fn derive_legacy_pair_verify_key_iv(shared_secret: &[u8]) -> ([u8; 16], [u8; 16]) {
    let mut key_hasher = HkdfSha512::new();
    key_hasher.update(b"Pair-Verify-AES-Key");
    key_hasher.update(shared_secret);
    let key_digest = key_hasher.finalize();

    let mut iv_hasher = HkdfSha512::new();
    iv_hasher.update(b"Pair-Verify-AES-IV");
    iv_hasher.update(shared_secret);
    let iv_digest = iv_hasher.finalize();

    let mut key = [0_u8; 16];
    key.copy_from_slice(&key_digest[..16]);
    let mut iv = [0_u8; 16];
    iv.copy_from_slice(&iv_digest[..16]);
    (key, iv)
}

pub(crate) fn increment_big_endian_counter(counter: &mut [u8]) {
    for byte in counter.iter_mut().rev() {
        let (value, carry) = byte.overflowing_add(1);
        *byte = value;
        if !carry {
            break;
        }
    }
}

pub(crate) fn build_legacy_pair_setup_client_proof(
    username: &str,
    salt: &[u8],
    controller_public: &[u8],
    receiver_public: &[u8],
    session_key: &[u8],
) -> Vec<u8> {
    let group = G2048::generator();
    let proof = compute_m1_rfc5054::<SrpSha1>(
        &group,
        true,
        username.as_bytes(),
        salt,
        controller_public,
        receiver_public,
        session_key,
    );
    proof.to_vec()
}

fn verify_legacy_pair_setup_server_proof(
    controller_public: &[u8],
    client_proof: &[u8],
    session_key: &[u8],
    server_proof: &[u8],
) -> Result<(), AirPlayError> {
    let mut hasher = SrpSha1::new();
    hasher.update(controller_public);
    hasher.update(client_proof);
    hasher.update(session_key);
    let expected_proof = hasher.finalize();
    if expected_proof.as_slice() == server_proof {
        return Ok(());
    }

    Err(AirPlayError::AuthenticationFailed {
        message: String::from(
            "pair-setup-pin server proof verification failed; check whether the PIN is correct",
        ),
    })
}

pub(crate) fn left_pad_legacy_srp_public(public_key: &[u8]) -> Result<Vec<u8>, AirPlayError> {
    let modulus_len = G2048::generator().params().modulus().to_be_bytes().len();
    if public_key.len() > modulus_len {
        return Err(AirPlayError::AuthenticationFailed {
            message: String::from(
                "pair-setup-pin SRP public key length exceeds the 2048-bit group range",
            ),
        });
    }

    let mut padded = vec![0_u8; modulus_len];
    let offset = modulus_len - public_key.len();
    padded[offset..].copy_from_slice(public_key);
    Ok(padded)
}

fn encrypt_legacy_pair_setup_public_key(
    session_key: &[u8],
    controller_public_key: &[u8; 32],
) -> Result<LegacyEncryptedPublicKey, AirPlayError> {
    let (key, iv) = derive_legacy_pair_setup_key_iv(session_key);
    let cipher = aes_gcm::AesGcm::<Aes128, aes::cipher::consts::U16>::new_from_slice(&key)
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from("failed to initialize legacy pair-setup AES-GCM"),
        })?;
    let nonce = aes_gcm::Nonce::<aes::cipher::consts::U16>::clone_from_slice(&iv);
    let mut ciphertext = controller_public_key.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(&nonce, b"", &mut ciphertext)
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from("failed to encrypt the legacy pair-setup public key"),
        })?;
    Ok(LegacyEncryptedPublicKey {
        ciphertext,
        tag: tag.into(),
    })
}

fn decrypt_legacy_pair_setup_public_key(
    session_key: &[u8],
    encrypted_public_key: &[u8],
    auth_tag: &[u8],
) -> Result<[u8; 32], AirPlayError> {
    let (key, mut iv) = derive_legacy_pair_setup_key_iv(session_key);
    increment_big_endian_counter(&mut iv);
    let cipher = aes_gcm::AesGcm::<Aes128, aes::cipher::consts::U16>::new_from_slice(&key)
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from("failed to initialize legacy pair-setup AES-GCM"),
        })?;
    let nonce = aes_gcm::Nonce::<aes::cipher::consts::U16>::clone_from_slice(&iv);
    let mut plaintext = encrypted_public_key.to_vec();
    let tag = aes_gcm::Tag::<aes::cipher::consts::U16>::from_slice(auth_tag);
    cipher
        .decrypt_in_place_detached(&nonce, b"", &mut plaintext, tag)
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from("failed to decrypt the legacy pair-setup receiver public key"),
        })?;
    decode_fixed_32(&plaintext, "legacy pair-setup receiver public key")
}

pub(crate) fn apply_legacy_pair_verify_finish_signature_keystream(
    verify_key: &[u8; 16],
    verify_iv: &[u8; 16],
    challenge: &[u8; 64],
    signature: &[u8; 64],
) -> [u8; 64] {
    let mut cipher = Ctr128BE::<Aes128>::new(verify_key.into(), verify_iv.into());
    let mut discarded = *challenge;
    cipher.apply_keystream(&mut discarded);
    let mut encrypted_signature = *signature;
    cipher.apply_keystream(&mut encrypted_signature);
    encrypted_signature
}

pub(crate) fn apply_legacy_pair_verify_signature_keystream(
    verify_key: &[u8; 16],
    verify_iv: &[u8; 16],
    signature: &[u8; 64],
) -> [u8; 64] {
    let mut cipher = Ctr128BE::<Aes128>::new(verify_key.into(), verify_iv.into());
    let mut transformed_signature = *signature;
    cipher.apply_keystream(&mut transformed_signature);
    transformed_signature
}

fn map_legacy_pair_verify_start_response(
    response: &RtspResponse,
) -> Result<LegacyPairVerifyStartReply, AirPlayError> {
    if !response.is_success() {
        return Err(map_pairing_http_failure(response, "/pair-verify"));
    }
    if response.body.len() != 96 {
        return Err(AirPlayError::Protocol {
            message: format!(
                "invalid legacy /pair-verify start response length: {}",
                response.body.len()
            ),
        });
    }
    let public_key = decode_fixed_32(&response.body[..32], "legacy /pair-verify public key")?;
    let challenge = response.body[32..]
        .try_into()
        .map_err(|_| AirPlayError::Protocol {
            message: String::from("legacy /pair-verify challenge must be 64 bytes"),
        })?;
    Ok(LegacyPairVerifyStartReply {
        public_key,
        challenge,
    })
}

fn verify_legacy_pair_verify_signature(
    receiver_curve_public: &[u8; 32],
    controller_curve_public: &[u8; 32],
    encrypted_signature: &[u8; 64],
    receiver_public_key: &[u8; 32],
    verify_key: &[u8; 16],
    verify_iv: &[u8; 16],
) -> Result<(), AirPlayError> {
    let signature = Signature::from_bytes(&apply_legacy_pair_verify_signature_keystream(
        verify_key,
        verify_iv,
        encrypted_signature,
    ));
    let receiver_verifying_key = VerifyingKey::from_bytes(receiver_public_key).map_err(|_| {
        AirPlayError::InvalidSession {
            message: String::from("mDNS pk is not a valid Ed25519 public key"),
        }
    })?;
    let mut signed_message = Vec::with_capacity(64);
    signed_message.extend_from_slice(receiver_curve_public);
    signed_message.extend_from_slice(controller_curve_public);
    receiver_verifying_key
        .verify(&signed_message, &signature)
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from("legacy /pair-verify receiver signature verification failed"),
        })
}

pub(crate) fn encrypt_pair_verify_payload(
    key_material: &[u8; 32],
    nonce_suffix: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, AirPlayError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key_material));
    let nonce = build_pair_verify_nonce(nonce_suffix)?;
    cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: &[],
            },
        )
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from("failed to build pair-verify encrypted payload"),
        })
}

fn decrypt_pair_verify_payload(
    key_material: &[u8; 32],
    nonce_suffix: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, AirPlayError> {
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key_material));
    let nonce = build_pair_verify_nonce(nonce_suffix)?;
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad: &[],
            },
        )
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from(
                "failed to decrypt the pair-verify response; check whether the saved pairing is still valid",
            ),
        })
}

fn build_prefixed_nonce(nonce_suffix: &[u8]) -> Result<Nonce, AirPlayError> {
    if nonce_suffix.len() > 8 {
        return Err(AirPlayError::Protocol {
            message: String::from("invalid pairing nonce suffix length"),
        });
    }
    let mut nonce = [0_u8; 12];
    let start = nonce.len().saturating_sub(nonce_suffix.len());
    nonce[start..].copy_from_slice(nonce_suffix);
    Ok(*Nonce::from_slice(&nonce))
}

fn build_pair_verify_nonce(nonce_suffix: &[u8]) -> Result<Nonce, AirPlayError> {
    build_prefixed_nonce(nonce_suffix)
}

pub(crate) fn decode_receiver_credentials(
    receiver_credentials: &ReceiverCredentials,
    device: &Receiver,
) -> Result<DecodedReceiverCredentials, AirPlayError> {
    let controller_ltsk = decode_hex_bytes(
        &receiver_credentials.controller_ltsk_hex,
        "controller_ltsk_hex",
    )?;
    let controller_secret_key: [u8; 32] =
        controller_ltsk
            .try_into()
            .map_err(|_| AirPlayError::InvalidSession {
                message: String::from(
                    "controller_ltsk_hex requires a 32-byte Ed25519 private key seed",
                ),
            })?;
    let controller_signing_key = SigningKey::from_bytes(&controller_secret_key);
    let controller_public_bytes = decode_hex_fixed::<32>(
        &receiver_credentials.controller_ltpk_hex,
        "controller_ltpk_hex",
    )?;
    if controller_signing_key.verifying_key().to_bytes() != controller_public_bytes {
        return Err(AirPlayError::InvalidSession {
            message: String::from("controller_ltpk_hex does not match controller_ltsk_hex"),
        });
    }

    let receiver_public_bytes =
        decode_hex_fixed::<32>(&receiver_credentials.receiver_ltpk_hex, "receiver_ltpk_hex")?;
    let receiver_verifying_key =
        VerifyingKey::from_bytes(&receiver_public_bytes).map_err(|_| {
            AirPlayError::InvalidSession {
                message: String::from("receiver_ltpk_hex is not a valid Ed25519 public key"),
            }
        })?;

    if let Some(device_pairing_id) = device.pairing_id.as_deref()
        && device_pairing_id != receiver_credentials.receiver_pairing_id
    {
        return Err(AirPlayError::InvalidSession {
            message: String::from(
                "saved receiver pairing id does not match the current mDNS pi/gid",
            ),
        });
    }

    if let Some(device_public_key) = device.receiver_public_key.as_deref() {
        let decoded_device_public_key = decode_mdns_public_key(device_public_key)?;
        if decoded_device_public_key != receiver_public_bytes {
            return Err(AirPlayError::InvalidSession {
                message: String::from(
                    "saved receiver public key does not match the current mDNS pk",
                ),
            });
        }
    }

    Ok(DecodedReceiverCredentials {
        auth_flow: receiver_credentials.auth_flow.clone(),
        controller_pairing_id: receiver_credentials.controller_pairing_id.clone(),
        controller_signing_key,
        receiver_pairing_id: receiver_credentials.receiver_pairing_id.clone(),
        receiver_verifying_key,
    })
}

fn decode_hex_fixed<const N: usize>(
    value: &str,
    field_name: &str,
) -> Result<[u8; N], AirPlayError> {
    let bytes = decode_hex_bytes(value, field_name)?;
    bytes.try_into().map_err(|_| AirPlayError::InvalidSession {
        message: format!("{field_name} requires {N} bytes of hex data"),
    })
}

fn decode_hex_bytes(value: &str, field_name: &str) -> Result<Vec<u8>, AirPlayError> {
    hex::decode(value).map_err(|_| AirPlayError::InvalidSession {
        message: format!("{field_name} is not a valid hex string"),
    })
}

fn verify_receiver_identity(
    receiver_credentials: &DecodedReceiverCredentials,
    controller_public: &[u8; 32],
    start_reply: &PairVerifyStartReply,
    verify_key: &[u8; 32],
) -> Result<(), AirPlayError> {
    let decrypted = decrypt_pair_verify_payload(
        verify_key,
        PAIR_VERIFY_M2_NONCE,
        &start_reply.encrypted_data,
    )?;
    let tlv = PairingTlv::parse(&decrypted)?;
    let identifier = std::str::from_utf8(tlv.require(PAIRING_TLV_IDENTIFIER, "identifier")?)
        .map_err(|_| AirPlayError::Protocol {
            message: String::from("identifier in pair-verify response is not valid UTF-8"),
        })?;
    if identifier != receiver_credentials.receiver_pairing_id {
        return Err(AirPlayError::AuthenticationFailed {
            message: String::from(
                "receiver identifier returned by pair-verify does not match the saved pairing record",
            ),
        });
    }

    let signature_bytes = decode_signature_bytes(tlv.require(PAIRING_TLV_SIGNATURE, "signature")?)?;
    let signature = Signature::from_bytes(&signature_bytes);
    let mut signed_message =
        Vec::with_capacity(32 + receiver_credentials.receiver_pairing_id.len() + 32);
    signed_message.extend_from_slice(&start_reply.public_key);
    signed_message.extend_from_slice(receiver_credentials.receiver_pairing_id.as_bytes());
    signed_message.extend_from_slice(controller_public);
    receiver_credentials
        .receiver_verifying_key
        .verify(&signed_message, &signature)
        .map_err(|_| AirPlayError::AuthenticationFailed {
            message: String::from(
                "pair-verify receiver signature verification failed; check whether the saved pairing record has expired",
            ),
        })
}

pub(crate) fn decode_signature_bytes(bytes: &[u8]) -> Result<[u8; 64], AirPlayError> {
    bytes.try_into().map_err(|_| AirPlayError::Protocol {
        message: String::from("pair-verify signature must be 64 bytes"),
    })
}

fn map_pair_verify_start_response(
    response: &RtspResponse,
) -> Result<PairVerifyStartReply, AirPlayError> {
    if !response.is_success() {
        return Err(map_pairing_http_failure(response, "/pair-verify"));
    }

    let tlv = PairingTlv::parse(&response.body)?;
    if let Some(error_code) = tlv.get(PAIRING_TLV_ERROR) {
        return Err(AirPlayError::AuthenticationFailed {
            message: format!("/pair-verify returned TLV error={error_code:02x?}"),
        });
    }
    let state = tlv.require_byte(PAIRING_TLV_STATE, "state")?;
    if state != PAIR_VERIFY_START_RESPONSE_STATE {
        return Err(AirPlayError::Protocol {
            message: format!("/pair-verify start response returned unexpected state={state}"),
        });
    }

    Ok(PairVerifyStartReply {
        public_key: decode_fixed_32(
            tlv.require(PAIRING_TLV_PUBLIC_KEY, "public key")?,
            "/pair-verify public key",
        )?,
        encrypted_data: tlv
            .require(PAIRING_TLV_ENCRYPTED_DATA, "encrypted data")?
            .to_vec(),
    })
}

fn map_pair_verify_finish_response(response: &RtspResponse) -> Result<(), AirPlayError> {
    if !response.is_success() {
        return Err(map_pairing_http_failure(response, "/pair-verify"));
    }

    let tlv = PairingTlv::parse(&response.body)?;
    if let Some(error_code) = tlv.get(PAIRING_TLV_ERROR) {
        return Err(AirPlayError::AuthenticationFailed {
            message: format!("/pair-verify finish step returned TLV error={error_code:02x?}"),
        });
    }
    let state = tlv.require_byte(PAIRING_TLV_STATE, "state")?;
    if state != PAIR_VERIFY_FINISH_RESPONSE_STATE {
        return Err(AirPlayError::Protocol {
            message: format!("/pair-verify finish response returned unexpected state={state}"),
        });
    }

    Ok(())
}

pub(crate) fn decode_fixed_32(bytes: &[u8], field_name: &str) -> Result<[u8; 32], AirPlayError> {
    bytes.try_into().map_err(|_| AirPlayError::Protocol {
        message: format!("{field_name} must be 32 bytes"),
    })
}

fn map_pairing_http_failure(response: &RtspResponse, path: &str) -> AirPlayError {
    match response.status.code {
        401 | 470 => AirPlayError::PairingRequired,
        403 => AirPlayError::AuthenticationFailed {
            message: format!("{path} was rejected by the receiver"),
        },
        code => AirPlayError::Protocol {
            message: format!("{path} returned unexpected status {code}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Cursor, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;
    use crate::audio::AudioFormat;
    use crate::pairing::{ReceiverAuthFlow, ReceiverCredentials};
    use crate::receiver::{
        AirPlayGeneration, AuthMethod, DeviceSupport, Receiver, ReceiverCapabilities, ReceiverKind,
    };
    use crate::rtsp::{RtspHeaders, RtspMethod, RtspRequest, RtspResponse, RtspStatus};
    use crate::session::{AirPlayError, ModernAirPlaySession, PreparedSession, SessionDescriptor};

    fn build_receiver(
        port: u16,
        receiver_kind: ReceiverKind,
        pairing_identity: Option<String>,
        receiver_public_key: Option<String>,
    ) -> Receiver {
        Receiver {
            id: String::from("speaker"),
            name: String::from("Speaker"),
            host: String::from("127.0.0.1"),
            port,
            generation: if receiver_kind.is_modern() {
                AirPlayGeneration::AirPlay2
            } else {
                AirPlayGeneration::AirPlay1
            },
            transport_profile: receiver_kind,
            support_level: DeviceSupport::Supported,
            auth_method: AuthMethod::None,
            pairing_identity,
            receiver_public_key,
            capabilities: ReceiverCapabilities::default(),
            ..Receiver::default()
        }
        .with_compat_fields()
    }

    #[test]
    fn modern_session_connects_for_modern_receiver() {
        let descriptor = SessionDescriptor::new(
            build_receiver(7000, ReceiverKind::ModernAirPlayAuth, None, None),
            AudioFormat::default(),
        );

        let session = ModernAirPlaySession::connect(&descriptor).unwrap();

        assert_eq!(
            session.descriptor().device.receiver_kind,
            ReceiverKind::ModernAirPlayAuth
        );
        assert_eq!(ModernAirPlaySession::transport_name(), "airplay2");
    }

    #[test]
    fn modern_session_request_builders_advance_cseq() {
        let descriptor = SessionDescriptor::new(
            build_receiver(7000, ReceiverKind::ModernAirPlayAuth, None, None),
            AudioFormat::default(),
        );
        let mut session = ModernAirPlaySession::connect(&descriptor).unwrap();

        let info = session.info_request();
        let pair_setup = session.pair_setup_request([1_u8, 2, 3]);
        let pair_verify = session.pair_verify_request([4_u8, 5]);

        assert_eq!(info.headers.get("CSeq"), Some("1"));
        assert_eq!(pair_setup.headers.get("CSeq"), Some("2"));
        assert_eq!(pair_verify.headers.get("CSeq"), Some("3"));
    }

    #[test]
    fn pair_pin_start_response_maps_success() {
        let response = RtspResponse {
            status: RtspStatus {
                code: 200,
                reason_phrase: String::from("OK"),
            },
            headers: RtspHeaders::new(),
            body: Vec::new(),
        };

        assert!(map_pair_pin_start_response(&response).is_ok());
    }

    #[test]
    fn pair_pin_start_response_maps_pairing_required() {
        let response = RtspResponse {
            status: RtspStatus {
                code: 470,
                reason_phrase: String::from("Connection Authorization Required"),
            },
            headers: RtspHeaders::new(),
            body: Vec::new(),
        };

        let error = map_pair_pin_start_response(&response).unwrap_err();

        assert_eq!(error, AirPlayError::PairingRequired);
    }

    #[test]
    fn request_pairing_pin_display_probes_info_and_pair_pin_start() {
        let (server, recorded_requests) = FakeRtspServer::spawn(vec![
            String::from("RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n"),
            String::from("RTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n"),
        ]);
        let descriptor = SessionDescriptor::new(
            build_receiver(server.port(), ReceiverKind::ModernAirPlayAuth, None, None),
            AudioFormat::default(),
        );

        PreparedSession::prepare(&descriptor)
            .unwrap()
            .request_pairing_pin_display()
            .unwrap();

        let requests = recorded_requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /info RTSP/1.0"));
        assert!(requests[1].starts_with("POST /pair-pin-start RTSP/1.0"));
    }

    #[test]
    fn modern_handshake_maps_info_status_to_pairing_required() {
        let (server, recorded_requests) = FakeRtspServer::spawn(vec![
            String::from("RTSP/1.0 401 Unauthorized\r\nCSeq: 1\r\n\r\n"),
            String::from("RTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n"),
        ]);
        let descriptor = SessionDescriptor::new(
            build_receiver(server.port(), ReceiverKind::ModernAirPlayAuth, None, None),
            AudioFormat::default(),
        );

        let error = ModernAirPlaySession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap_err();

        assert_eq!(error, AirPlayError::PairingRequired);
        let requests = recorded_requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /info RTSP/1.0"));
        assert!(requests[1].starts_with("POST /pair-pin-start RTSP/1.0"));
    }

    #[test]
    fn pair_pin_start_470_is_treated_as_pairing_required() {
        let (server, _recorded_requests) = FakeRtspServer::spawn(vec![
            String::from("RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n"),
            String::from("RTSP/1.0 470 Connection Authorization Required\r\nCSeq: 2\r\n\r\n"),
        ]);
        let descriptor = SessionDescriptor::new(
            build_receiver(server.port(), ReceiverKind::ModernAirPlayAuth, None, None),
            AudioFormat::default(),
        );

        let error = ModernAirPlaySession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap_err();

        assert_eq!(error, AirPlayError::PairingRequired);
    }

    #[test]
    fn modern_handshake_without_credentials_probes_pair_pin_start_even_if_info_returns_ok() {
        let (server, recorded_requests) = FakeRtspServer::spawn(vec![
            String::from("RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n"),
            String::from("RTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n"),
        ]);
        let descriptor = SessionDescriptor::new(
            build_receiver(server.port(), ReceiverKind::ModernAirPlayAuth, None, None),
            AudioFormat::default(),
        );

        let error = ModernAirPlaySession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap_err();

        assert_eq!(error, AirPlayError::PairingRequired);
        let requests = recorded_requests
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /info RTSP/1.0"));
        assert!(requests[1].starts_with("POST /pair-pin-start RTSP/1.0"));
    }

    struct PairSetupPinContext {
        verifier: Vec<u8>,
        expected_user: String,
        salt: [u8; 16],
        server_secret: [u8; 48],
        server_public: Vec<u8>,
    }

    fn respond_modern_pairing_info(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
    ) {
        let info_request = read_rtsp_message(reader).unwrap();
        requests.push(info_request);
        stream
            .write_all(b"RTSP/1.0 401 Unauthorized\r\nCSeq: 1\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
    }

    fn respond_pair_pin_start(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
    ) {
        let request = read_rtsp_message(reader).unwrap();
        requests.push(request);
        stream
            .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 2\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
    }

    fn respond_pair_setup_pin_start(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
        expected_pin: &str,
    ) -> PairSetupPinContext {
        let request_bytes = read_rtsp_message_bytes(reader).unwrap();
        let request = parse_request_from_bytes(&request_bytes).unwrap();
        requests.push(String::from_utf8_lossy(&request_bytes).into_owned());
        let dictionary = Value::from_reader(Cursor::new(&request.body))
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(
            dictionary.get("method"),
            Some(&Value::String(String::from("pin")))
        );
        let controller_pairing_id = dictionary.get("user").and_then(Value::as_string).unwrap();
        assert_eq!(controller_pairing_id.len(), 17);
        assert_eq!(controller_pairing_id.matches(':').count(), 5);

        let srp_client = srp::ClientG2048::<SrpSha1>::new();
        let srp_server = srp::ServerG2048::<SrpSha1>::new();
        let salt = *b"0123456789abcdef";
        let verifier = srp_client.compute_verifier(
            controller_pairing_id.as_bytes(),
            expected_pin.as_bytes(),
            &salt,
        );
        let mut server_secret = [0_u8; 48];
        OsRng.fill_bytes(&mut server_secret);
        let server_public = left_pad_legacy_srp_public(
            &srp_server.compute_public_ephemeral(&server_secret, &verifier),
        )
        .unwrap();

        let mut response = Dictionary::new();
        response.insert(String::from("salt"), Value::Data(salt.to_vec()));
        response.insert(String::from("pk"), Value::Data(server_public.clone()));
        let body = encode_binary_plist(&Value::Dictionary(response)).unwrap();
        stream
            .write_all(&rtsp_response_with_binary_body(3, &body))
            .unwrap();
        stream.flush().unwrap();

        PairSetupPinContext {
            verifier,
            expected_user: controller_pairing_id.to_string(),
            salt,
            server_secret,
            server_public,
        }
    }

    fn respond_pair_setup_pin_verify(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
        start: &PairSetupPinContext,
    ) -> Vec<u8> {
        let request_bytes = read_rtsp_message_bytes(reader).unwrap();
        let request = parse_request_from_bytes(&request_bytes).unwrap();
        requests.push(String::from_utf8_lossy(&request_bytes).into_owned());
        let dictionary = Value::from_reader(Cursor::new(&request.body))
            .unwrap()
            .into_dictionary()
            .unwrap();
        let controller_public = dictionary.get("pk").and_then(Value::as_data).unwrap();
        let controller_proof = dictionary.get("proof").and_then(Value::as_data).unwrap();
        let srp_server = srp::ServerG2048::<SrpSha1>::new();
        let server_verifier = srp_server
            .process_reply(
                start.expected_user.as_bytes(),
                &start.salt,
                &start.server_secret,
                &start.verifier,
                controller_public,
            )
            .unwrap();
        let server_session_key = derive_legacy_pair_setup_session_key(server_verifier.key());
        let expected_controller_proof = build_legacy_pair_setup_client_proof(
            &start.expected_user,
            &start.salt,
            controller_public,
            &start.server_public,
            &server_session_key,
        );
        assert_eq!(controller_proof, expected_controller_proof.as_slice());

        let mut hasher = SrpSha1::new();
        hasher.update(controller_public);
        hasher.update(controller_proof);
        hasher.update(&server_session_key);
        let mut response = Dictionary::new();
        response.insert(
            String::from("proof"),
            Value::Data(hasher.finalize().to_vec()),
        );
        let body = encode_binary_plist(&Value::Dictionary(response)).unwrap();
        stream
            .write_all(&rtsp_response_with_binary_body(4, &body))
            .unwrap();
        stream.flush().unwrap();
        server_session_key
    }

    fn respond_pair_setup_pin_exchange(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
        shared_secret: &[u8],
    ) {
        let request_bytes = read_rtsp_message_bytes(reader).unwrap();
        let request = parse_request_from_bytes(&request_bytes).unwrap();
        requests.push(String::from_utf8_lossy(&request_bytes).into_owned());
        let dictionary = Value::from_reader(Cursor::new(&request.body))
            .unwrap()
            .into_dictionary()
            .unwrap();
        let encrypted_public_key = dictionary.get("epk").and_then(Value::as_data).unwrap();
        let auth_tag = dictionary.get("authTag").and_then(Value::as_data).unwrap();
        assert_eq!(encrypted_public_key.len(), 32);
        assert_eq!(auth_tag.len(), 16);

        let (key, mut iv) = derive_legacy_pair_setup_key_iv(shared_secret);
        let cipher =
            aes_gcm::AesGcm::<Aes128, aes::cipher::consts::U16>::new_from_slice(&key).unwrap();
        let request_nonce = aes_gcm::Nonce::<aes::cipher::consts::U16>::clone_from_slice(&iv);
        let mut controller_public_key = encrypted_public_key.to_vec();
        cipher
            .decrypt_in_place_detached(
                &request_nonce,
                b"",
                &mut controller_public_key,
                aes_gcm::Tag::<aes::cipher::consts::U16>::from_slice(auth_tag),
            )
            .unwrap();
        assert_eq!(controller_public_key.len(), 32);

        increment_big_endian_counter(&mut iv);
        let response_nonce = aes_gcm::Nonce::<aes::cipher::consts::U16>::clone_from_slice(&iv);
        let receiver_public_key = SigningKey::from_bytes(&[9_u8; 32])
            .verifying_key()
            .to_bytes();
        let mut encrypted_receiver_public_key = receiver_public_key.to_vec();
        let response_tag = cipher
            .encrypt_in_place_detached(&response_nonce, b"", &mut encrypted_receiver_public_key)
            .unwrap();

        let mut response = Dictionary::new();
        response.insert(
            String::from("epk"),
            Value::Data(encrypted_receiver_public_key),
        );
        response.insert(String::from("authTag"), Value::Data(response_tag.to_vec()));
        let body = encode_binary_plist(&Value::Dictionary(response)).unwrap();
        stream
            .write_all(&rtsp_response_with_binary_body(5, &body))
            .unwrap();
        stream.flush().unwrap();
    }

    struct LegacyPairVerifyContext {
        receiver_curve_public: [u8; 32],
        controller_curve_public: [u8; 32],
        challenge: [u8; 64],
        verify_key: [u8; 16],
        verify_iv: [u8; 16],
        controller_signing_public: [u8; 32],
    }

    fn respond_legacy_pair_verify_start(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
        receiver_signing_key: &SigningKey,
    ) -> LegacyPairVerifyContext {
        let request_bytes = read_rtsp_message_bytes(reader).unwrap();
        let request = parse_request_from_bytes(&request_bytes).unwrap();
        requests.push(String::from_utf8_lossy(&request_bytes).into_owned());
        assert_eq!(request.body.len(), 68);
        assert_eq!(&request.body[..4], &[1, 0, 0, 0]);
        let controller_curve_public =
            decode_fixed_32(&request.body[4..36], "controller curve public").unwrap();
        let controller_signing_public =
            decode_fixed_32(&request.body[36..68], "controller signing public").unwrap();

        let receiver_secret = EphemeralSecret::random_from_rng(OsRng);
        let receiver_curve_public = X25519PublicKey::from(&receiver_secret).to_bytes();
        let shared_secret =
            receiver_secret.diffie_hellman(&X25519PublicKey::from(controller_curve_public));
        let (verify_key, verify_iv) = derive_legacy_pair_verify_key_iv(shared_secret.as_bytes());
        let mut signed_message = Vec::with_capacity(64);
        signed_message.extend_from_slice(&receiver_curve_public);
        signed_message.extend_from_slice(&controller_curve_public);
        let signature = receiver_signing_key.sign(&signed_message).to_bytes();
        let encrypted_signature =
            apply_legacy_pair_verify_signature_keystream(&verify_key, &verify_iv, &signature);

        let mut response_body = Vec::with_capacity(96);
        response_body.extend_from_slice(&receiver_curve_public);
        response_body.extend_from_slice(&encrypted_signature);
        stream
            .write_all(&rtsp_response_with_binary_body(6, &response_body))
            .unwrap();
        stream.flush().unwrap();

        LegacyPairVerifyContext {
            receiver_curve_public,
            controller_curve_public,
            challenge: encrypted_signature,
            verify_key,
            verify_iv,
            controller_signing_public,
        }
    }

    fn respond_legacy_pair_verify_finish(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
        context: &LegacyPairVerifyContext,
    ) {
        let request_bytes = read_rtsp_message_bytes(reader).unwrap();
        let request = parse_request_from_bytes(&request_bytes).unwrap();
        requests.push(String::from_utf8_lossy(&request_bytes).into_owned());
        assert_eq!(request.body.len(), 68);
        assert_eq!(&request.body[..4], &[0, 0, 0, 0]);

        let encrypted_signature = decode_signature_bytes(&request.body[4..68]).unwrap();
        let signature = apply_legacy_pair_verify_finish_signature_keystream(
            &context.verify_key,
            &context.verify_iv,
            &context.challenge,
            &encrypted_signature,
        );
        let signature = ed25519_dalek::Signature::from_bytes(&signature);
        let controller_verifying_key =
            VerifyingKey::from_bytes(&context.controller_signing_public).unwrap();
        let mut signed_message = Vec::with_capacity(64);
        signed_message.extend_from_slice(&context.controller_curve_public);
        signed_message.extend_from_slice(&context.receiver_curve_public);
        controller_verifying_key
            .verify(&signed_message, &signature)
            .unwrap();

        stream
            .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 7\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
    }

    fn spawn_modern_pairing_server(pin: &str) -> (u16, mpsc::Receiver<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let port_listener = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();
        let expected_pin = pin.to_string();

        thread::spawn(move || {
            let (mut stream, _) = port_listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut requests = Vec::new();

            respond_modern_pairing_info(&mut stream, &mut reader, &mut requests);
            respond_pair_pin_start(&mut stream, &mut reader, &mut requests);
            let start = respond_pair_setup_pin_start(
                &mut stream,
                &mut reader,
                &mut requests,
                &expected_pin,
            );
            let shared_secret =
                respond_pair_setup_pin_verify(&mut stream, &mut reader, &mut requests, &start);
            respond_pair_setup_pin_exchange(
                &mut stream,
                &mut reader,
                &mut requests,
                &shared_secret,
            );
            let receiver_signing_key = SigningKey::from_bytes(&[9_u8; 32]);
            let context = respond_legacy_pair_verify_start(
                &mut stream,
                &mut reader,
                &mut requests,
                &receiver_signing_key,
            );
            respond_legacy_pair_verify_finish(&mut stream, &mut reader, &mut requests, &context);

            tx.send(requests).unwrap();
        });

        (port, rx)
    }

    fn respond_raop_requests(
        stream: &mut TcpStream,
        reader: &mut BufReader<TcpStream>,
        requests: &mut Vec<String>,
        responses: &[Vec<u8>],
    ) {
        for response in responses {
            let request = read_rtsp_message(reader).unwrap();
            requests.push(request);
            stream.write_all(response).unwrap();
            stream.flush().unwrap();
        }
    }

    fn spawn_modern_authenticated_server(
        receiver_signing_key: SigningKey,
        receiver_pairing_id: String,
    ) -> (u16, mpsc::Receiver<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let port_listener = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = port_listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut requests = Vec::new();

            let info_request = read_rtsp_message(&mut reader).unwrap();
            requests.push(info_request);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();

            let pair_verify_start_request = read_rtsp_message_bytes(&mut reader).unwrap();
            let pair_verify_start = parse_request_from_bytes(&pair_verify_start_request).unwrap();
            requests.push(String::from_utf8_lossy(&pair_verify_start_request).into_owned());
            let pair_verify_start_tlv = PairingTlv::parse(&pair_verify_start.body).unwrap();
            let controller_public_bytes = decode_fixed_32(
                pair_verify_start_tlv
                    .require(PAIRING_TLV_PUBLIC_KEY, "public key")
                    .unwrap(),
                "controller public key",
            )
            .unwrap();
            let controller_public = X25519PublicKey::from(controller_public_bytes);
            let receiver_secret = EphemeralSecret::random_from_rng(OsRng);
            let receiver_public = X25519PublicKey::from(&receiver_secret);
            let shared_secret = receiver_secret.diffie_hellman(&controller_public);
            let verify_key = derive_hkdf_sha512(
                shared_secret.as_bytes(),
                PAIR_VERIFY_ENCRYPT_SALT,
                PAIR_VERIFY_ENCRYPT_INFO,
            )
            .unwrap();
            let mut verify_plaintext = PairingTlv::new();
            verify_plaintext.push(PAIRING_TLV_IDENTIFIER, receiver_pairing_id.as_bytes());
            let mut signed_message = Vec::new();
            signed_message.extend_from_slice(receiver_public.as_bytes());
            signed_message.extend_from_slice(receiver_pairing_id.as_bytes());
            signed_message.extend_from_slice(controller_public.as_bytes());
            let receiver_signature = receiver_signing_key.sign(&signed_message);
            verify_plaintext.push(PAIRING_TLV_SIGNATURE, receiver_signature.to_bytes());
            let encrypted_data = encrypt_pair_verify_payload(
                &verify_key,
                PAIR_VERIFY_M2_NONCE,
                &verify_plaintext.encode(),
            )
            .unwrap();
            let mut pair_verify_response_tlv = PairingTlv::new();
            pair_verify_response_tlv.push(PAIRING_TLV_STATE, [2_u8]);
            pair_verify_response_tlv.push(PAIRING_TLV_PUBLIC_KEY, *receiver_public.as_bytes());
            pair_verify_response_tlv.push(PAIRING_TLV_ENCRYPTED_DATA, encrypted_data);
            let pair_verify_start_body = pair_verify_response_tlv.encode();
            stream
                .write_all(&rtsp_response_with_binary_body(2, &pair_verify_start_body))
                .unwrap();
            stream.flush().unwrap();

            let pair_verify_finish_request = read_rtsp_message(&mut reader).unwrap();
            requests.push(pair_verify_finish_request);
            let mut pair_verify_finish_tlv = PairingTlv::new();
            pair_verify_finish_tlv.push(PAIRING_TLV_STATE, [PAIR_VERIFY_FINISH_RESPONSE_STATE]);
            let pair_verify_finish_body = pair_verify_finish_tlv.encode();
            stream
                .write_all(&rtsp_response_with_binary_body(3, &pair_verify_finish_body))
                .unwrap();
            stream.flush().unwrap();

            respond_raop_requests(
                &mut stream,
                &mut reader,
                &mut requests,
                &[
                    b"RTSP/1.0 200 OK\r\nCSeq: 4\r\nContent-Length: 0\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nCSeq: 5\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nCSeq: 6\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port=5200;control_port=5201;timing_port=5202\r\nSession: modernbeef;timeout=60\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nSession: modernbeef\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nSession: modernbeef\r\n\r\n".to_vec(),
                ],
            );

            tx.send(requests).unwrap();
        });

        (port, rx)
    }

    fn spawn_legacy_pin_authenticated_server(
        receiver_signing_key: SigningKey,
    ) -> (u16, mpsc::Receiver<Vec<String>>) {
        spawn_legacy_pin_authenticated_server_with_auth_setup_status(receiver_signing_key, 200)
    }

    fn spawn_legacy_pin_authenticated_server_with_auth_setup_status(
        receiver_signing_key: SigningKey,
        auth_setup_status: u16,
    ) -> (u16, mpsc::Receiver<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let port_listener = listener.try_clone().unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = port_listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut requests = Vec::new();

            let info_request = read_rtsp_message(&mut reader).unwrap();
            requests.push(info_request);
            stream
                .write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();

            let context = respond_legacy_pair_verify_start(
                &mut stream,
                &mut reader,
                &mut requests,
                &receiver_signing_key,
            );
            respond_legacy_pair_verify_finish(&mut stream, &mut reader, &mut requests, &context);

            let auth_setup_response = if auth_setup_status == 404 {
                b"RTSP/1.0 404 Not Found\r\nCSeq: 4\r\nContent-Length: 0\r\n\r\n".to_vec()
            } else {
                b"RTSP/1.0 200 OK\r\nCSeq: 4\r\nContent-Length: 0\r\n\r\n".to_vec()
            };
            respond_raop_requests(
                &mut stream,
                &mut reader,
                &mut requests,
                &[
                    auth_setup_response,
                    b"RTSP/1.0 200 OK\r\nCSeq: 5\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nCSeq: 6\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nTransport: RTP/AVP/UDP;unicast;mode=record;server_port=5200;control_port=5201;timing_port=5202\r\nSession: legacybeef;timeout=60\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nSession: legacybeef\r\n\r\n".to_vec(),
                    b"RTSP/1.0 200 OK\r\nSession: legacybeef\r\n\r\n".to_vec(),
                ],
            );

            tx.send(requests).unwrap();
        });

        (port, rx)
    }

    fn build_authenticated_descriptor(
        auth_flow: ReceiverAuthFlow,
        port: u16,
        controller_signing_key: &SigningKey,
        controller_pairing_id: &str,
        receiver_pairing_id: &str,
        receiver_ltpk: [u8; 32],
    ) -> SessionDescriptor {
        SessionDescriptor::new(
            build_receiver(
                port,
                ReceiverKind::ModernAirPlayAuth,
                Some(receiver_pairing_id.to_string()),
                Some(hex::encode(receiver_ltpk)),
            ),
            AudioFormat::default(),
        )
        .with_receiver_credentials(ReceiverCredentials {
            auth_flow,
            controller_pairing_id: controller_pairing_id.to_string(),
            controller_ltpk_hex: hex::encode(controller_signing_key.verifying_key().to_bytes()),
            controller_ltsk_hex: hex::encode(controller_signing_key.to_bytes()),
            receiver_pairing_id: receiver_pairing_id.to_string(),
            receiver_ltpk_hex: hex::encode(receiver_ltpk),
        })
    }

    fn assert_modern_authenticated_request_flow(requests: &[String]) {
        assert_eq!(requests.len(), 9);
        assert!(requests[0].starts_with("GET /info RTSP/1.0"));
        assert!(requests[1].starts_with("POST /pair-verify RTSP/1.0"));
        assert!(requests[2].starts_with("POST /pair-verify RTSP/1.0"));
        assert!(requests[3].starts_with("POST /auth-setup RTSP/1.0"));
        assert!(requests[4].starts_with("OPTIONS "));
        assert!(requests[5].starts_with("ANNOUNCE "));
        assert!(requests[6].starts_with("SETUP "));
        assert!(requests[7].starts_with("RECORD "));
        assert!(requests[8].starts_with("TEARDOWN "));
    }

    fn assert_legacy_pin_authenticated_request_flow(requests: &[String]) {
        assert_eq!(requests.len(), 9);
        assert!(requests[0].starts_with("GET /info RTSP/1.0"));
        assert!(requests[1].starts_with("POST /pair-verify RTSP/1.0"));
        assert!(requests[2].starts_with("POST /pair-verify RTSP/1.0"));
        assert!(requests[3].starts_with("POST /auth-setup RTSP/1.0"));
        assert!(requests[4].starts_with("OPTIONS "));
        assert!(requests[5].starts_with("ANNOUNCE "));
        assert!(requests[6].starts_with("SETUP "));
        assert!(requests[7].starts_with("RECORD "));
        assert!(requests[8].starts_with("TEARDOWN "));
    }

    #[test]
    fn legacy_pin_handshake_restores_authentication_and_continues_raop_flow() {
        let mut controller_seed = [0_u8; 32];
        OsRng.fill_bytes(&mut controller_seed);
        let controller_signing_key = SigningKey::from_bytes(&controller_seed);
        let mut receiver_seed = [0_u8; 32];
        OsRng.fill_bytes(&mut receiver_seed);
        let receiver_signing_key = SigningKey::from_bytes(&receiver_seed);
        let receiver_ltpk = receiver_signing_key.verifying_key().to_bytes();
        let receiver_pairing_id = "receiver-id";
        let controller_pairing_id = "controller-id";

        let (port, rx) = spawn_legacy_pin_authenticated_server(receiver_signing_key);
        let descriptor = build_authenticated_descriptor(
            ReceiverAuthFlow::LegacyPin,
            port,
            &controller_signing_key,
            controller_pairing_id,
            receiver_pairing_id,
            receiver_ltpk,
        );

        let connection = ModernAirPlaySession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap();
        let transport = connection.stream_transport().unwrap();
        assert_eq!(transport.audio_target.port(), 5200);
        connection.teardown().unwrap();

        let requests = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_legacy_pin_authenticated_request_flow(&requests);
    }

    #[test]
    fn legacy_pin_handshake_tolerates_auth_setup_not_found_and_continues_raop_flow() {
        let mut controller_seed = [0_u8; 32];
        OsRng.fill_bytes(&mut controller_seed);
        let controller_signing_key = SigningKey::from_bytes(&controller_seed);
        let mut receiver_seed = [0_u8; 32];
        OsRng.fill_bytes(&mut receiver_seed);
        let receiver_signing_key = SigningKey::from_bytes(&receiver_seed);
        let receiver_ltpk = receiver_signing_key.verifying_key().to_bytes();
        let receiver_pairing_id = "receiver-id";
        let controller_pairing_id = "controller-id";

        let (port, rx) =
            spawn_legacy_pin_authenticated_server_with_auth_setup_status(receiver_signing_key, 404);
        let descriptor = build_authenticated_descriptor(
            ReceiverAuthFlow::LegacyPin,
            port,
            &controller_signing_key,
            controller_pairing_id,
            receiver_pairing_id,
            receiver_ltpk,
        );

        let connection = ModernAirPlaySession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap();
        let transport = connection.stream_transport().unwrap();
        assert_eq!(transport.audio_target.port(), 5200);
        connection.teardown().unwrap();

        let requests = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_legacy_pin_authenticated_request_flow(&requests);
    }

    #[test]
    fn modern_pair_with_pin_returns_receiver_credentials() {
        let receiver_signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let (port, rx) = spawn_modern_pairing_server("1234");
        let descriptor = SessionDescriptor::new(
            build_receiver(
                port,
                ReceiverKind::ModernAirPlayAuth,
                Some(String::from("receiver-id")),
                Some(hex::encode(receiver_signing_key.verifying_key().to_bytes())),
            ),
            AudioFormat::default(),
        );

        let credentials = ModernAirPlaySession::connect(&descriptor)
            .unwrap()
            .pair_with_pin("1234")
            .unwrap();

        assert_eq!(credentials.receiver_pairing_id, "receiver-id");
        assert_eq!(credentials.receiver_ltpk_hex.len(), 64);
        assert_eq!(credentials.controller_ltpk_hex.len(), 64);
        assert_eq!(credentials.controller_ltsk_hex.len(), 64);

        let requests = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(requests.len(), 7);
        assert!(requests[0].starts_with("GET /info RTSP/1.0"));
        assert!(requests[1].starts_with("POST /pair-pin-start RTSP/1.0"));
        assert!(requests[2].starts_with("POST /pair-setup-pin RTSP/1.0"));
        assert!(requests[3].starts_with("POST /pair-setup-pin RTSP/1.0"));
        assert!(requests[4].starts_with("POST /pair-setup-pin RTSP/1.0"));
        assert!(requests[5].starts_with("POST /pair-verify RTSP/1.0"));
        assert!(requests[6].starts_with("POST /pair-verify RTSP/1.0"));
    }

    #[test]
    fn modern_handshake_restores_authentication_and_continues_raop_flow() {
        let mut controller_seed = [0_u8; 32];
        OsRng.fill_bytes(&mut controller_seed);
        let controller_signing_key = SigningKey::from_bytes(&controller_seed);
        let mut receiver_seed = [0_u8; 32];
        OsRng.fill_bytes(&mut receiver_seed);
        let receiver_signing_key = SigningKey::from_bytes(&receiver_seed);
        let receiver_ltpk = receiver_signing_key.verifying_key().to_bytes();
        let receiver_pairing_id = "receiver-id";
        let controller_pairing_id = "controller-id";

        let (port, rx) = spawn_modern_authenticated_server(
            receiver_signing_key,
            receiver_pairing_id.to_string(),
        );
        let descriptor = build_authenticated_descriptor(
            ReceiverAuthFlow::Modern,
            port,
            &controller_signing_key,
            controller_pairing_id,
            receiver_pairing_id,
            receiver_ltpk,
        );

        let connection = ModernAirPlaySession::connect(&descriptor)
            .unwrap()
            .handshake()
            .unwrap();
        let transport = connection.stream_transport().unwrap();
        assert_eq!(transport.audio_target.port(), 5200);
        connection.teardown().unwrap();

        let requests = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_modern_authenticated_request_flow(&requests);
    }

    struct FakeRtspServer {
        listener: TcpListener,
    }

    impl FakeRtspServer {
        fn spawn(responses: Vec<String>) -> (Self, mpsc::Receiver<Vec<String>>) {
            Self::spawn_bytes(responses.into_iter().map(String::into_bytes).collect())
        }

        fn spawn_bytes(responses: Vec<Vec<u8>>) -> (Self, mpsc::Receiver<Vec<String>>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port_listener = listener.try_clone().unwrap();
            let (tx, rx) = mpsc::channel();

            thread::spawn(move || {
                let (mut stream, _) = port_listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut requests = Vec::new();

                for response in responses {
                    let request = read_rtsp_message(&mut reader).unwrap();
                    requests.push(request);
                    stream.write_all(&response).unwrap();
                    stream.flush().unwrap();
                }

                tx.send(requests).unwrap();
            });

            (Self { listener }, rx)
        }

        fn port(&self) -> u16 {
            self.listener.local_addr().unwrap().port()
        }
    }

    fn rtsp_response_with_binary_body(cseq: u32, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn read_rtsp_message(reader: &mut BufReader<TcpStream>) -> Result<String, std::io::Error> {
        let raw = read_rtsp_message_bytes(reader)?;
        Ok(String::from_utf8_lossy(&raw).into_owned())
    }

    fn read_rtsp_message_bytes(
        reader: &mut BufReader<TcpStream>,
    ) -> Result<Vec<u8>, std::io::Error> {
        let mut raw = Vec::new();
        let mut content_length = 0_usize;

        loop {
            let mut line = String::new();
            let bytes_read = reader.read_line(&mut line)?;
            if bytes_read == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed",
                ));
            }

            let normalized = line.trim_end_matches(['\r', '\n']);
            raw.extend_from_slice(normalized.as_bytes());
            raw.extend_from_slice(b"\r\n");

            if normalized.is_empty() {
                break;
            }

            if let Some((name, value)) = normalized.split_once(':')
                && name.trim().eq_ignore_ascii_case("Content-Length")
            {
                content_length = value.trim().parse::<usize>().unwrap();
            }
        }

        if content_length > 0 {
            let mut body = vec![0_u8; content_length];
            reader.read_exact(&mut body)?;
            raw.extend_from_slice(&body);
        }

        Ok(raw)
    }

    fn parse_request_from_bytes(raw: &[u8]) -> Result<RtspRequest, String> {
        let separator = b"\r\n\r\n";
        let (head, body) = match raw
            .windows(separator.len())
            .position(|window| window == separator)
        {
            Some(index) => (&raw[..index], &raw[index + separator.len()..]),
            None => (raw, &[][..]),
        };
        let head_text = std::str::from_utf8(head)
            .map_err(|_| String::from("RTSP request head is not valid UTF-8"))?;
        let mut lines = head_text.lines();
        let request_line = lines
            .next()
            .ok_or_else(|| String::from("missing RTSP request line"))?;
        let mut parts = request_line.split_whitespace();
        let method = match parts.next() {
            Some("OPTIONS") => RtspMethod::Options,
            Some("ANNOUNCE") => RtspMethod::Announce,
            Some("SETUP") => RtspMethod::Setup,
            Some("RECORD") => RtspMethod::Record,
            Some("TEARDOWN") => RtspMethod::Teardown,
            Some("GET") => RtspMethod::Get,
            Some("POST") => RtspMethod::Post,
            Some(other) => return Err(format!("unknown RTSP method: {other}")),
            None => return Err(String::from("missing RTSP method")),
        };
        let uri = parts
            .next()
            .ok_or_else(|| String::from("missing RTSP URI"))?;
        let mut request = RtspRequest::new(method, uri);
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                request.headers.insert(name.trim(), value.trim());
            }
        }
        request.body = body.to_vec();
        Ok(request)
    }
}
