use std::time::SystemTime;
use rustls::{Certificate, HandshakeSignatureValid, ResolvesClientCert, RootCertStore, ServerCertVerified, ServerCertVerifier, SignatureScheme, TLSError};
use rustls::internal::msgs::handshake::DigitallySignedStruct;
use rustls::sign::CertifiedKey;


pub struct NoVerifyTLS;

impl ServerCertVerifier for NoVerifyTLS {
    fn verify_server_cert(
        &self,
        _roots: &RootCertStore,
        _presented_certs: &[Certificate],
        _dns_name: webpki::DNSNameRef,
        _ocsp_response: &[u8],
    ) -> Result<ServerCertVerified, TLSError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &Certificate,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TLSError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &Certificate,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TLSError> {
        Ok(HandshakeSignatureValid::assertion())
    }
}

impl ResolvesClientCert for NoVerifyTLS {
    fn resolve(
        &self,
        _acceptable_issuers: &[&[u8]],
        _sigschemes: &[SignatureScheme],
    ) -> Option<CertifiedKey> {
        None
    }

    fn has_certs(&self) -> bool {
        false
    }
}
