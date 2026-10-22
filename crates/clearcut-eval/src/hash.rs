// Written by Kone Tshivhinda & Claude
//! SHA-256 of a document, as the 64 lowercase hex digits a label records.

use sha2::{Digest, Sha256};
use std::io::{self, Read};

/// The SHA-256 of `bytes`, as 64 lowercase hex digits.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes).iter().copied())
}

/// The SHA-256 and length of everything `reader` yields, read in small pieces
/// so a large document is never held in memory.
pub fn sha256_hex_of_reader(mut reader: impl Read) -> io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut length = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        length += read as u64;
    }
    Ok((hex(hasher.finalize().iter().copied()), length))
}

fn hex(bytes: impl Iterator<Item = u8>) -> String {
    bytes.map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_test_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn reading_in_pieces_gives_the_same_answer_as_hashing_at_once() {
        let data: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
        let (streamed, length) = sha256_hex_of_reader(&data[..]).unwrap();
        assert_eq!(streamed, sha256_hex(&data));
        assert_eq!(length, 200_000);
        assert_eq!(sha256_hex_of_reader(&b""[..]).unwrap(), (sha256_hex(b""), 0));
    }

    #[test]
    fn a_read_error_is_passed_on() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("boom"))
            }
        }
        assert!(sha256_hex_of_reader(Broken).is_err());
    }
}
