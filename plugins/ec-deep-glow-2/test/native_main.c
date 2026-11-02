/* Native sanitizer harness: ./a.out W H p0 p1 ... (flat params). Builds a neon test frame, renders, checks output. */
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <math.h>
int ec_render(float *buf, int W, int H, const double *params, int np, double time, double scale);
static void *blocks[4096]; static int nb;
void *ec_native_alloc(uint64_t bytes) { void *p = malloc(bytes ? bytes : 1); if (nb < 4096) blocks[nb++] = p; return p; }
int main(int argc, char **argv) {
    int W = atoi(argv[1]), H = atoi(argv[2]), np = argc - 3;
    double *P = malloc(sizeof(double) * (np > 0 ? np : 1));
    for (int i = 0; i < np; i++) P[i] = strtod(argv[3 + i], 0);
    float *buf = malloc((size_t)W * H * 16);
    for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) {
        float *p = buf + ((size_t)y * W + x) * 4; float v = 0.02f + 0.03f * y / H;
        p[0] = v * .6f; p[1] = v * .8f; p[2] = v; p[3] = (x % 7 == 0) ? 0.5f : 1.f;
        if (abs(x - W / 2) < 3 && abs(y - H / 2) < 3) p[0] = p[1] = p[2] = 20.f;
        if (x == W / 4 && y == H / 3) { p[0] = 50.f; p[1] = 2.f; p[2] = 0.1f; }
    }
    int rc = ec_render(buf, W, H, P, np, 1.25, 1.0), bad = 0;
    for (size_t i = 0; i < (size_t)W * H * 4; i++) if (!isfinite(buf[i]) || buf[i] < 0.f) bad++;
    printf("rc=%d bad=%d\n", rc, bad);
    for (int i = 0; i < nb; i++) free(blocks[i]);
    free(buf); free(P);
    return (rc != 0 && rc != 2) || bad;
}
