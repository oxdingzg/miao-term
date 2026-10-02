//! Import the real `.ppk` fixtures (generated with `puttygen` 0.81) and check
//! that the resulting OpenSSH key carries the same public key and is encrypted.

use miao_term_keys::import_ppk;

const NEW: &str = "a-fresh-passphrase";

fn testdata(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(name)
}

/// `ed25519_v2_enc.ppk` -> `ed25519_plain.pub`.
fn public_fixture(ppk: &str) -> String {
    let base = ppk.split('_').next().unwrap();
    std::fs::read_to_string(testdata(&format!("{base}_plain.pub"))).unwrap()
}

fn check(ppk: &str, old: &str) {
    let text = std::fs::read_to_string(testdata(ppk)).unwrap();
    let imported = import_ppk(&text, old, NEW).unwrap();
    let want = public_fixture(ppk);
    assert_eq!(
        imported.public_openssh.trim(),
        want.trim(),
        "{ppk}: public key"
    );

    let encrypted = ssh_key::PrivateKey::from_openssh(imported.private_openssh.as_str()).unwrap();
    assert!(encrypted.is_encrypted(), "{ppk}: output is not encrypted");
    let decrypted = encrypted.decrypt(NEW).unwrap();
    assert_eq!(
        decrypted.public_key().to_openssh().unwrap().trim(),
        want.trim(),
        "{ppk}: round-tripped public key"
    );
}

#[test]
fn imports_every_key_type_and_ppk_version() {
    for key in ["ed25519", "rsa", "ecdsa"] {
        check(&format!("{key}_v2.ppk"), "");
        check(&format!("{key}_v2_enc.ppk"), "mtty-test-v2");
        check(&format!("{key}_v3.ppk"), "");
        check(&format!("{key}_v3_enc.ppk"), "mtty-test-v3");
    }
}

#[test]
fn a_wrong_passphrase_is_rejected() {
    let text = std::fs::read_to_string(testdata("ed25519_v2_enc.ppk")).unwrap();
    assert!(import_ppk(&text, "not-the-passphrase", NEW).is_err());
    let text3 = std::fs::read_to_string(testdata("rsa_v3_enc.ppk")).unwrap();
    assert!(import_ppk(&text3, "not-the-passphrase", NEW).is_err());
}

#[test]
fn the_output_is_never_unencrypted() {
    let text = std::fs::read_to_string(testdata("ed25519_v2.ppk")).unwrap();
    let err = match import_ppk(&text, "", "") {
        Ok(_) => panic!("an empty new passphrase must be refused"),
        Err(e) => e,
    };
    assert!(err.contains("must not be empty"), "{err}");
}

#[test]
fn a_corrupt_mac_is_refused() {
    let text = std::fs::read_to_string(testdata("ed25519_v2.ppk")).unwrap();
    let tampered = text.replace("Comment: mtty", "Comment: evil");
    assert!(import_ppk(&tampered, "", NEW).is_err());
}
