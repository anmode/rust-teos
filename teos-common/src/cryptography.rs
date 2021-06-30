use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Error, PublicKey, SecretKey};
use bitcoin::util::psbt::serialize::{Deserialize, Serialize};
use bitcoin::{Transaction, Txid};
use chacha20poly1305::aead::{Aead, NewAead};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use lightning::util::message_signing;

/// Enum representing the possible errors when decrypting an encrypted blob.
#[derive(Debug)]
pub enum DecryptingError {
    AED(chacha20poly1305::aead::Error),
    Encode(bitcoin::consensus::encode::Error),
}

/// Shadows message_signing::sign.
pub fn sign(msg: &[u8], sk: SecretKey) -> Result<String, Error> {
    message_signing::sign(msg, sk)
}

/// Shadows message_signing::verify.
pub fn verify(msg: &[u8], sig: &str, pk: PublicKey) -> bool {
    match message_signing::recover_pk(msg, sig) {
        Ok(x) => x == pk,
        Err(_) => false,
    }
}

/// Shadows message_signing::recover_pk.
pub fn recover_pk(msg: &[u8], sig: &str) -> Result<PublicKey, Error> {
    message_signing::recover_pk(msg, sig)
}

/// Encrypts a given message (the penalty transaction) under a given secret (the dispute txid) using chacha20poly1305 with [0; 12] as IV.
pub fn encrypt(
    message: &Transaction,
    secret: &Txid,
) -> Result<Vec<u8>, chacha20poly1305::aead::Error> {
    // Defaults is [0; 12]
    let nonce = Nonce::default();
    let _k = sha256::Hash::hash(&secret);
    let key = Key::from_slice(&_k);

    let cypher = ChaCha20Poly1305::new(key);
    cypher.encrypt(&nonce, message.serialize().as_ref())
}

/// Decrypts an encrypted blob of data using a given secret (the dispute txid) using chacha20poly1305 with [0; 12] as IV. The result is expected to
/// be a penalty transaction.
pub fn decrypt(encrypted_blob: &Vec<u8>, secret: &Txid) -> Result<Transaction, DecryptingError> {
    // Defaults is [0; 12]
    let nonce = Nonce::default();
    let _k = sha256::Hash::hash(&secret);
    let key = Key::from_slice(&_k);

    let cypher = ChaCha20Poly1305::new(key);

    match cypher.decrypt(&nonce, encrypted_blob.as_ref()) {
        Ok(tx_bytes) => match Transaction::deserialize(&tx_bytes) {
            Ok(tx) => Ok(tx),
            Err(e) => Err(DecryptingError::Encode(e)),
        },
        Err(e) => Err(DecryptingError::AED(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::{hashes::hex::FromHex, util::psbt::serialize::Deserialize};

    const HEX_TX: &str = "010000000001010000000000000000000000000000000000000000000000000000000000000000ffffffff54038e830a1b4d696e656420627920416e74506f6f6c373432c2005b005e7a0ae3fabe6d6d7841cd582ead8ea5dd8e3de1173cae6fcd2a53c7362ebb7fb6f815604fe07cbe0200000000000000ac0e060005f90000ffffffff04d9476026000000001976a91411dbe48cc6b617f9c6adaf4d9ed5f625b1c7cb5988ac0000000000000000266a24aa21a9ed7248c6efddd8d99bfddd7f499f0b915bffa8253003cc934df1ff14a81301e2340000000000000000266a24b9e11b6d7054937e13f39529d6ad7e685e9dd4efa426f247d5f5a5bed58cdddb2d0fa60100000000000000002b6a2952534b424c4f434b3a054a68aa5368740e8b3e3c67bce45619c2cfd07d4d4f0936a5612d2d0034fa0a0120000000000000000000000000000000000000000000000000000000000000000000000000";
    const HEX_TXID: &str = "d6ac4a5e61657c4c604dcde855a1db74ec6b3e54f32695d72c5e11c7761ea1b4";
    const ENC_BLOB: &str = "f64d730654738fdbcd9e65068be17bc1abb44e74f8977985cce48e77209cf97292c862e4eb7190aedc6c53ceddda6871a3988d1d9608e2d0dd7a1f59769e410618a7029001479ac3b9d699b11a08b0ccb04e56bfee88461d9cd3207623a4a543996dd3805323c93cd62069636305aaf159e9cca1063ad1f097c16fb3c2ebbcf09be96512c5d7c195c684569cbe8b7979870b04cada9806b7610569c66021afcc63f46dd4af75716950c4de094334cdf7d9e532820afe29d2621dd79920c7e0ecc10853517dd84ca9d699f712c229e86954c227cba1d0fc87c8d48ac05e2de8a6bc980afdfafcd7064e411c8d76065c06cc7f233e869eaff5bd8ccb5d8f0090d91a8f017355cc115863356ecf06cdda9b309096ea766d033dbd4f70a789a5b03138cfc7e2900a79bb465abf07a7ac45c41b4b30c008d4b299aad9d001cf45acd07e47cdd63c3b13d4b0788b041735225b5db1a43a2142311f695478168e31deb260702976fd70d0724ded84a7c3f89b";

    #[test]
    fn test_encrypt() {
        let expected_enc_blob = Vec::from_hex(ENC_BLOB).unwrap();
        let tx_bytes = Vec::from_hex(HEX_TX).unwrap();

        let tx = Transaction::deserialize(&tx_bytes).unwrap();
        let txid = Txid::from_hex(HEX_TXID).unwrap();
        assert_eq!(encrypt(&tx, &txid).unwrap(), expected_enc_blob);
    }

    #[test]
    fn test_decrypt() {
        let expected_tx = Transaction::deserialize(&Vec::from_hex(HEX_TX).unwrap()).unwrap();

        let encrypted_blob = Vec::from_hex(ENC_BLOB).unwrap();
        let txid = Txid::from_hex(HEX_TXID).unwrap();
        assert_eq!(decrypt(&&encrypted_blob, &txid).unwrap(), expected_tx);
    }
}
