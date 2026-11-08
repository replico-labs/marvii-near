// WebAuthn only. The passkey's private key never leaves the authenticator;
// the page keeps the credential ID and public key, which are not secret.

const hex = (b: ArrayBuffer | Uint8Array) =>
  [...new Uint8Array(b instanceof Uint8Array ? b : new Uint8Array(b))].map((x) => x.toString(16).padStart(2, '0')).join('');
export const fromHex = (h: string) => new Uint8Array(h.match(/../g)!.map((x) => parseInt(x, 16)));
const b64url = (b: ArrayBuffer) => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const unb64url = (s: string) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));

export type Passkey = { credentialId: string; publicKey: string };

// SubjectPublicKeyInfo header for a P-256 key (id-ecPublicKey, prime256v1),
// followed by the 65-byte uncompressed point.
const P256_SPKI_PREFIX = '3059301306072a8648ce3d020106082a8648ce3d030107034200';

export async function createPasskey(name: string): Promise<Passkey> {
  const cred = (await navigator.credentials.create({
    publicKey: {
      rp: { id: location.hostname, name: 'Marvii' },
      user: { id: crypto.getRandomValues(new Uint8Array(16)), name, displayName: name },
      challenge: crypto.getRandomValues(new Uint8Array(32)),
      pubKeyCredParams: [{ type: 'public-key', alg: -7 }], // ES256 only: the wallet checks P-256
      authenticatorSelection: { residentKey: 'required', userVerification: 'required' },
      attestation: 'none',
    },
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error('No passkey was created');
  const r = cred.response as AuthenticatorAttestationResponse;
  if (r.getPublicKeyAlgorithm() !== -7) throw new Error('This passkey is not P-256');
  const spki = hex(r.getPublicKey()!);
  if (!spki.startsWith(P256_SPKI_PREFIX) || spki.length !== P256_SPKI_PREFIX.length + 130) {
    throw new Error('Unexpected public key format');
  }
  return { credentialId: b64url(cred.rawId), publicKey: spki.slice(P256_SPKI_PREFIX.length) };
}

/** Signs a 32-byte challenge. Returns the parts the wallet's proof needs. */
export async function signChallenge(pk: Passkey, challengeHex: string) {
  const cred = (await navigator.credentials.get({
    publicKey: {
      rpId: location.hostname,
      challenge: fromHex(challengeHex),
      allowCredentials: [{ type: 'public-key', id: unb64url(pk.credentialId) }],
      userVerification: 'required',
    },
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error('Cancelled');
  const r = cred.response as AuthenticatorAssertionResponse;
  return {
    authenticatorData: hex(r.authenticatorData),
    clientDataJSON: new TextDecoder().decode(r.clientDataJSON),
    signature: hex(r.signature),
  };
}
