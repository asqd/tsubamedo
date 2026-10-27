#!/bin/sh
# Test certificates for tests/network.rs: a CA, a server for api.test, localhost and 127.0.0.1,
# a client in PEM, in PKCS#12 and with an encrypted key (passphrase "kot"). Valid 100 years.
set -e
cd "$(dirname "$0")"
O=${OPENSSL:-/usr/bin/openssl}
$O req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -days 36500 -subj "/CN=TsubameDo Test CA"
$O req -newkey rsa:2048 -nodes -keyout server.key -out server.csr -subj "/CN=api.test"
printf "subjectAltName=DNS:api.test,DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n" > server.ext
$O x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out server.pem -days 36500 -extfile server.ext
$O req -newkey rsa:2048 -nodes -keyout client.key -out client.csr -subj "/CN=client"
printf "extendedKeyUsage=clientAuth\n" > client.ext
$O x509 -req -in client.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out client.pem -days 36500 -extfile client.ext
$O pkcs12 -export -in client.pem -inkey client.key -out client.p12 -passout pass:kot
$O pkey -in client.key -aes256 -passout pass:kot -out client-encrypted.key
rm -f server.csr client.csr server.ext client.ext ca.srl ca.key
