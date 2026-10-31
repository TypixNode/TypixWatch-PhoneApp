/* Exposes the watch's w1a_sec.c on stdin/stdout, one request per line, hex arguments.
 * Compiled by tests/interop_c.rs from the sources next to this repository, so what
 * answers is the code the watch and the Mac tool run, not a copy. */
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "monocypher.h"
#include "w1a_sec.h"

#define MAX (1 << 21)

static char line[MAX];

static size_t unhex(uint8_t *out, const char *s)
{
	size_t n = 0;

	if (!strcmp(s, "-"))	/* empty */
		return 0;
	for (; s[0] && s[1]; s += 2) {
		unsigned v;

		sscanf(s, "%2x", &v);
		out[n++] = v;
	}
	return n;
}

static void put(const uint8_t *p, size_t n)
{
	for (size_t i = 0; i < n; i++)
		printf("%02x", p[i]);
	printf(n ? " " : "- ");
}

int main(void)
{
	static uint8_t a[MAX], b[MAX], c[MAX], d[MAX], e[MAX], out[MAX];
	char *args[8];

	while (fgets(line, sizeof line, stdin)) {
		int n = 0;
		char *cmd = strtok(line, " \n");

		while (n < 8 && (args[n] = strtok(NULL, " \n")))
			n++;
		if (!cmd)
			continue;
		if (!strcmp(cmd, "map")) {
			unhex(a, args[0]);
			crypto_elligator_map(out, a);
			put(out, 32);
		} else if (!strcmp(cmd, "share")) {
			unhex(a, args[0]);
			unhex(b, args[2]);
			w1a_cpace_share(out, a, args[1], b);
			put(out, 32);
		} else if (!strcmp(cmd, "finish")) {
			struct w1a_cipher c1, c2;

			unhex(a, args[0]);
			unhex(b, args[1]);
			unhex(c, args[2]);
			unhex(d, args[3]);
			unhex(e, args[4]);
			if (w1a_cpace_finish(&c1, &c2, a, b, c, d, e)) {
				printf("degenerate");
			} else {
				put(c1.k, 32);
				put(c2.k, 32);
			}
		} else if (!strcmp(cmd, "ik")) {
			/* Both ends, then one record each way. */
			static struct w1a_noise i, r;
			struct w1a_cipher i1, i2, r1, r2;
			uint8_t pub[32], m1[W1A_IK1_LEN], m2[W1A_IK2_LEN], ping[20], pong[20];
			size_t pn;

			unhex(a, args[0]);
			unhex(b, args[1]);
			unhex(c, args[2]);
			unhex(d, args[3]);
			pn = unhex(e, args[4]);
			w1a_x25519_public(pub, b);
			w1a_ik_init(&i, a, pub, c, e, pn);
			w1a_ik_init(&r, b, NULL, d, e, pn);
			w1a_ik_write1(&i, m1);
			if (w1a_ik_read1(&r, m1))
				printf("reject1 ");
			w1a_ik_write2(&r, m2);
			if (w1a_ik_read2(&i, m2))
				printf("reject2 ");
			w1a_noise_split(&i, &i1, &i2);
			w1a_noise_split(&r, &r1, &r2);
			w1a_seal(&i1, NULL, 0, "ping", 4, ping);
			w1a_seal(&r2, NULL, 0, "pong", 4, pong);
			put(m1, sizeof m1);
			put(m2, sizeof m2);
			put(ping, sizeof ping);
			put(pong, sizeof pong);
		} else if (!strcmp(cmd, "manifest")) {
			size_t mn;

			unhex(a, args[3]);
			unhex(b, args[4]);	/* the name, hex so it can be UTF-8 */
			b[strlen(args[4]) / 2] = 0;
			mn = w1a_manifest(out, atoi(args[0]), strtoul(args[1], NULL, 10),
					  strtoul(args[2], NULL, 10), a, (char *)b);
			put(out, mn);
		} else if (!strcmp(cmd, "sign")) {
			uint8_t secret[64], pub[32], sig[64];
			size_t mn;

			unhex(a, args[0]);
			mn = unhex(b, args[1]);
			w1a_sign_keypair(secret, pub, a);
			w1a_sign(sig, secret, b, mn);
			put(pub, 32);
			put(sig, 64);
			if (w1a_verify(sig, pub, b, mn))
				printf("verify-failed");
		} else if (!strcmp(cmd, "verify")) {
			size_t mn;

			unhex(a, args[0]);
			unhex(b, args[1]);
			mn = unhex(c, args[2]);
			printf("%d", w1a_verify(a, b, c, mn) == 0);
		} else if (!strcmp(cmd, "fingerprint")) {
			char fp[17];

			unhex(a, args[0]);
			w1a_fingerprint(fp, a);
			printf("%s", fp);
		} else if (!strcmp(cmd, "public")) {
			unhex(a, args[0]);
			w1a_x25519_public(out, a);
			put(out, 32);
		} else if (!strcmp(cmd, "seal")) {
			struct w1a_cipher k;
			size_t pn;

			unhex(k.k, args[0]);
			k.n = strtoull(args[1], NULL, 10);
			pn = unhex(b, args[2]);
			w1a_seal(&k, NULL, 0, b, pn, out);
			put(out, pn + W1A_TAG);
		} else if (!strcmp(cmd, "open")) {
			struct w1a_cipher k;
			size_t cn;

			unhex(k.k, args[0]);
			k.n = strtoull(args[1], NULL, 10);
			cn = unhex(b, args[2]);
			if (w1a_open(&k, NULL, 0, b, cn, out))
				printf("bad");
			else
				put(out, cn - W1A_TAG);
		} else if (!strcmp(cmd, "sha256")) {
			struct w1a_sha256 h;
			size_t dn = unhex(a, args[0]);

			w1a_sha256_init(&h);
			w1a_sha256_update(&h, a, dn);
			w1a_sha256_final(&h, out);
			put(out, 32);
		} else {
			printf("unknown");
		}
		printf("\n");
		fflush(stdout);
	}
	return 0;
}
