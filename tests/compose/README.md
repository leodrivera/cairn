# Compose tests

`run.sh` brings up each Compose example of the install guide (`examples/compose/`) against an image and checks
what the guide promises: the container gets healthy, the first-admin command works as typed, people sign in, files
land on the host with the right owner, backups restore, and the sign-in routes behind Traefik, Tinyauth and an OIDC
provider work end to end.

```bash
docker build -t cairn:ci .
tests/compose/run.sh cairn:ci            # every example
tests/compose/run.sh cairn:ci quick      # one
```

It needs Docker with Compose v2.24 or later, `curl`, `jq`, `openssl`, `python3` and `script`, and the loopback
ports 18080, 18081 and 18443. Everything runs from a copy in a temporary folder under projects named
`cairn-test-*`, which are taken down with their volumes at the end. No container named `cairn` may exist before a
run, because the examples use that name.

Hosts are `files.example.com`, `auth.example.com` and `id.example.com`. A throwaway CA made for each run signs a
certificate for them; `curl` reaches Traefik on `127.0.0.1:18443` and nothing resolves outside the test.

## Overrides

Each example runs with `override/<example>.yml`, which changes only what an offline test cannot use. The keys it
may change are listed in `override/<example>.allow`, and the check `override_only_allowed` fails when the merged
configuration differs anywhere else:

- `image`: the image under test instead of the published one.
- published ports: loopback ports, so a run never takes 80, 443 or 8080.
- Traefik's flags: the Let's Encrypt flags out and the test certificate in, and nothing else (`traefik_flags_match`).
- bind mounts for the test certificate and CA.
- values named in the test for each example (for example the OIDC issuer pointing at a local Keycloak).
