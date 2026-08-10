use std::net::IpAddr;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::model::SshError;

const MAX_SOCKS_DOMAIN: usize = 255;

pub(crate) fn validate_endpoint(host: &str, port: u16, field: &str) -> Result<(), SshError> {
    if host.is_empty()
        || host.len() > 255
        || host.chars().any(|character| {
            character.is_control() || character.is_whitespace() || character == '\0'
        })
        || port == 0
    {
        return Err(SshError::InvalidRequest(format!("{field} is invalid")));
    }
    Ok(())
}

pub(crate) fn validate_bind_host(host: &str) -> Result<(), SshError> {
    validate_endpoint(host, 1, "forward bind host")
}

pub(crate) async fn socks5_connect<S>(stream: &mut S) -> Result<(String, u16), SshError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let version = stream.read_u8().await.map_err(|_| SshError::Closed)?;
    let method_count = stream.read_u8().await.map_err(|_| SshError::Closed)? as usize;
    if version != 5 || method_count == 0 || method_count > 32 {
        return Err(SshError::InvalidRequest(
            "SOCKS5 greeting is invalid".into(),
        ));
    }
    let mut methods = vec![0; method_count];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|_| SshError::Closed)?;
    if !methods.contains(&0) {
        stream
            .write_all(&[5, 0xff])
            .await
            .map_err(|_| SshError::Closed)?;
        return Err(SshError::InvalidRequest(
            "SOCKS5 authentication is unsupported".into(),
        ));
    }
    stream
        .write_all(&[5, 0])
        .await
        .map_err(|_| SshError::Closed)?;

    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|_| SshError::Closed)?;
    if header[0] != 5 || header[1] != 1 || header[2] != 0 {
        send_socks5_failure(stream, 7).await?;
        return Err(SshError::InvalidRequest(
            "SOCKS5 request is unsupported".into(),
        ));
    }
    let host = match header[3] {
        1 => {
            let mut bytes = [0; 4];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| SshError::Closed)?;
            IpAddr::from(bytes).to_string()
        }
        3 => {
            let length = stream.read_u8().await.map_err(|_| SshError::Closed)? as usize;
            if length == 0 || length > MAX_SOCKS_DOMAIN {
                send_socks5_failure(stream, 8).await?;
                return Err(SshError::InvalidRequest("SOCKS5 domain is invalid".into()));
            }
            let mut bytes = vec![0; length];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| SshError::Closed)?;
            String::from_utf8(bytes)
                .map_err(|_| SshError::InvalidRequest("SOCKS5 domain is invalid".into()))?
        }
        4 => {
            let mut bytes = [0; 16];
            stream
                .read_exact(&mut bytes)
                .await
                .map_err(|_| SshError::Closed)?;
            IpAddr::from(bytes).to_string()
        }
        _ => {
            send_socks5_failure(stream, 8).await?;
            return Err(SshError::InvalidRequest(
                "SOCKS5 address type is unsupported".into(),
            ));
        }
    };
    let port = stream.read_u16().await.map_err(|_| SshError::Closed)?;
    if port == 0 {
        send_socks5_failure(stream, 8).await?;
        return Err(SshError::InvalidRequest("SOCKS5 port is invalid".into()));
    }
    Ok((host, port))
}

pub(crate) async fn send_socks5_success<S>(stream: &mut S) -> Result<(), SshError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|_| SshError::Closed)
}

pub(crate) async fn send_socks5_failure<S>(stream: &mut S, code: u8) -> Result<(), SshError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .map_err(|_| SshError::Closed)
}

#[cfg(test)]
mod tests {
    use super::socks5_connect;
    use tokio::io::duplex;

    #[tokio::test]
    async fn parses_domain_connect_request() {
        let (mut client, mut server) = duplex(1024);
        let writer = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(
                &mut client,
                &[
                    5, 1, 0, 5, 1, 0, 3, 12, b'e', b'x', b'a', b'm', b'p', b'l', b'e', b'.', b't',
                    b'e', b's', b't', 0, 22,
                ],
            )
            .await
            .unwrap();
            let mut response = [0; 2];
            tokio::io::AsyncReadExt::read_exact(&mut client, &mut response)
                .await
                .unwrap();
            assert_eq!(response, [5, 0]);
        });
        assert_eq!(
            socks5_connect(&mut server).await.unwrap(),
            ("example.test".into(), 22)
        );
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn rejects_authentication_methods_other_than_no_auth() {
        let (mut client, mut server) = duplex(64);
        let writer = tokio::spawn(async move {
            tokio::io::AsyncWriteExt::write_all(&mut client, &[5, 1, 2])
                .await
                .unwrap();
            let mut response = [0; 2];
            tokio::io::AsyncReadExt::read_exact(&mut client, &mut response)
                .await
                .unwrap();
            assert_eq!(response, [5, 0xff]);
        });
        assert!(socks5_connect(&mut server).await.is_err());
        writer.await.unwrap();
    }
}
