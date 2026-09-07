use std::fmt;

use zeroize::Zeroizing;

macro_rules! secret_key_type {
    ($name:ident) => {
        pub struct $name(Zeroizing<[u8; 32]>);

        impl $name {
            pub fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(Zeroizing::new(bytes))
            }

            pub fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    };
}

secret_key_type!(RootKey);
secret_key_type!(SecretKey);
