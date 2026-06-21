# TLS Trust Anchors

Each `.der` file is a single-anchor DER-encoded CA certificate selected at
build time by the `tls-ca-*` cargo feature.  Only one may be active at once.

## Anchor Inventory

| File | Feature flag | Key type | Subject | notAfter | Source |
|------|-------------|----------|---------|----------|--------|
| `private.der` | `tls-ca-private` (default) | ECDSA P-256 | CN=ViCell Private CA | 2036-06-18 | self-generated (see below) |
| `amazon-root-ca3.der` | `tls-ca-amazon` | ECDSA P-256 | CN=Amazon Root CA 3 | 2040-05-26 | https://www.amazontrust.com/repository/AmazonRootCA3.pem |
| `isrg-x2.der` | `tls-ca-letsencrypt` | ECDSA P-384 | CN=ISRG Root X2 | 2040-09-17 | https://letsencrypt.org/certs/isrg-root-x2.pem |

## Replacing `private.der` (production deployments)

`private.der` is a self-signed ECDSA P-256 CA generated during development.
For production you must replace it with your fleet's actual CA:

```sh
# 1. Generate production CA (keep the key offline / in HSM)
openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out fleet-ca.key
openssl req -new -x509 -key fleet-ca.key -out fleet-ca.pem -days 3650 \
    -subj "//CN=My Fleet CA"
# 2. Convert to DER and replace
openssl x509 -in fleet-ca.pem -out cells/services/net/roots/private.der -outform DER
# 3. Rebuild
cargo build -p service-net --release
```

## Refreshing public roots

Public roots rotate infrequently (notAfter above is 2040).  To refresh:

```sh
# Amazon Root CA 3
curl -sL https://www.amazontrust.com/repository/AmazonRootCA3.pem | \
    openssl x509 -outform DER -out cells/services/net/roots/amazon-root-ca3.der

# ISRG Root X2
curl -sL https://letsencrypt.org/certs/isrg-root-x2.pem | \
    openssl x509 -outform DER -out cells/services/net/roots/isrg-x2.der
```

Verify after refresh:
```sh
openssl x509 -in roots/private.der -inform DER -noout -dates
```
