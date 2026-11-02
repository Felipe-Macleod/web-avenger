//! Token OAuth do itch.io guardado cifrado.
//!
//! O token fica em `~/.config/web-avenger/itch-token.enc` (permissão 0600; no Windows,
//! `%APPDATA%\web-avenger\`), cifrado com ChaCha20-Poly1305. A chave (32 bytes aleatórios)
//! fica no chaveiro do sistema (Secret Service: gnome-keyring, KWallet...; no Windows, o
//! Gerenciador de Credenciais), protegida pelo login da sessão. Sem
//! chaveiro a conexão é recusada: guardar a chave ao lado do arquivo não protegeria nada.
//!
//! Só o processo principal usa este módulo; o token nunca chega ao JavaScript.

use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    fs, io,
    io::Write,
    path::PathBuf,
};

const KEYRING_SERVICE: &str = "web-avenger";
const KEYRING_ACCOUNT: &str = "itch-token-key";
/// Formato do arquivo: `versão(1) ‖ nonce(12) ‖ texto cifrado + tag(16)`.
const FORMAT_VERSION: u8 = 1;
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub access_token: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub user: Option<ItchUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItchUser {
    pub id: u64,
    pub username: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("web-avenger").join("itch-token.enc"))
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(keyring_error)
}

fn keyring_error(err: keyring::Error) -> String {
    match err {
        keyring::Error::NoStorageAccess(e) | keyring::Error::PlatformFailure(e) => {
            crate::i18n::tf("keyring.unavailable", &[("error", &e.to_string())])
        }
        other => crate::i18n::tf("keyring.error", &[("error", &other.to_string())]),
    }
}

/// Chave de criptografia do chaveiro; com `create`, gera uma nova se ainda não existir.
fn key(create: bool) -> Result<Option<[u8; 32]>, String> {
    let entry = keyring_entry()?;
    match entry.get_secret() {
        Ok(secret) => secret
            .try_into()
            .map(Some)
            .map_err(|_| crate::i18n::t("keyring.bad_key")),
        Err(keyring::Error::NoEntry) if create => {
            let mut key = [0u8; 32];
            getrandom::getrandom(&mut key).map_err(|e| format!("sem fonte de aleatoriedade: {e}"))?;
            entry.set_secret(&key).map_err(keyring_error)?;
            Ok(Some(key))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(keyring_error(err)),
    }
}

/// Confirma que o chaveiro funciona (e cria a chave), antes de iniciar o login.
pub fn ensure_key() -> Result<(), String> {
    key(true).map(|_| ())
}

pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|e| format!("sem fonte de aleatoriedade: {e}"))?;
    let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| "falha ao cifrar o token".to_string())?;
    let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
    out.push(FORMAT_VERSION);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    let (version, rest) = data.split_first().ok_or("arquivo do token vazio")?;
    if *version != FORMAT_VERSION || rest.len() < NONCE_LEN {
        return Err("formato do arquivo do token desconhecido".into());
    }
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| crate::i18n::t("keyring.decrypt"))
}

pub fn save(data: &TokenData) -> Result<(), String> {
    let key = key(true)?.ok_or("chave indisponível")?;
    let json = serde_json::to_vec(data).map_err(|e| e.to_string())?;
    let encrypted = encrypt(&key, &json)?;

    let path = path().ok_or("diretório de configuração não encontrado")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Grava num temporário 0600 e renomeia: o arquivo nunca fica legível por outros
    // nem pela metade. (No Windows a ACL do perfil já restringe o arquivo ao usuário.)
    let tmp = path.with_extension("enc.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&encrypted).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// `Ok(None)` quando não há conexão salva.
pub fn load() -> Result<Option<TokenData>, String> {
    let Some(path) = path() else {
        return Ok(None);
    };
    let data = match fs::read(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    let key = key(false)?.ok_or_else(|| crate::i18n::t("keyring.no_key"))?;
    let json = decrypt(&key, &data)?;
    serde_json::from_slice(&json).map(Some).map_err(|e| e.to_string())
}

/// Apaga o arquivo e a chave do chaveiro.
pub fn clear() -> Result<(), String> {
    if let Some(path) = path() {
        match fs::remove_file(path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err.to_string()),
            _ => {}
        }
    }
    match keyring_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(keyring_error(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cifra_e_decifra() {
        let key = [7u8; 32];
        let secret = br#"{"access_token":"segredo-123"}"#;
        let encrypted = encrypt(&key, secret).unwrap();
        assert_eq!(encrypted[0], FORMAT_VERSION);
        assert!(!encrypted.windows(9).any(|w| w == b"segredo-1"), "token em texto no arquivo");
        assert_eq!(decrypt(&key, &encrypted).unwrap(), secret);
        // Nonce aleatório: cifrar de novo dá outro resultado.
        assert_ne!(encrypt(&key, secret).unwrap(), encrypted);
    }

    #[test]
    fn rejeita_chave_errada_e_arquivo_alterado() {
        let encrypted = encrypt(&[1u8; 32], b"token").unwrap();
        assert!(decrypt(&[2u8; 32], &encrypted).is_err());
        let mut tampered = encrypted.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(decrypt(&[1u8; 32], &tampered).is_err());
        assert!(decrypt(&[1u8; 32], &[]).is_err());
        assert!(decrypt(&[1u8; 32], &[9, 1, 2]).is_err());
    }
}
