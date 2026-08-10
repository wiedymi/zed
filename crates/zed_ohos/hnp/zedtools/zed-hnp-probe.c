#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#ifndef PATH_MAX
#define PATH_MAX 4096
#endif

static int parse_number(const char *value, long minimum, long maximum, long *result)
{
    char *end = NULL;
    errno = 0;
    long parsed = strtol(value, &end, 10);
    if (errno != 0 || end == value || *end != '\0' || parsed < minimum || parsed > maximum) {
        return -1;
    }
    *result = parsed;
    return 0;
}

static int sleep_milliseconds(long milliseconds)
{
    struct timespec requested = {
        .tv_sec = milliseconds / 1000,
        .tv_nsec = (milliseconds % 1000) * 1000000,
    };
    while (nanosleep(&requested, &requested) != 0) {
        if (errno != EINTR) {
            return -1;
        }
    }
    return 0;
}

int main(int argc, char **argv)
{
    long exit_code = 0;
    long sleep_ms = 0;
    const char *stderr_text = "zed-hnp-probe-stderr";
    for (int index = 1; index < argc; index++) {
        if (strcmp(argv[index], "--exit-code") == 0 && index + 1 < argc) {
            if (parse_number(argv[++index], 0, 125, &exit_code) != 0) {
                fprintf(stderr, "invalid --exit-code\n");
                return 2;
            }
        } else if (strcmp(argv[index], "--sleep-ms") == 0 && index + 1 < argc) {
            if (parse_number(argv[++index], 0, 3600000, &sleep_ms) != 0) {
                fprintf(stderr, "invalid --sleep-ms\n");
                return 2;
            }
        } else if (strcmp(argv[index], "--stderr") == 0 && index + 1 < argc) {
            stderr_text = argv[++index];
        }
    }

    char cwd[PATH_MAX];
    if (getcwd(cwd, sizeof(cwd)) == NULL) {
        fprintf(stderr, "getcwd failed: %s\n", strerror(errno));
        return 3;
    }

    const char *token = getenv("ZED_HNP_PROBE_TOKEN");
    printf("protocol=zed-hnp-probe-v1\n");
    printf("cwd=%s\n", cwd);
    printf("token=%s\n", token == NULL ? "" : token);
    printf("argc=%d\n", argc);
    for (int index = 0; index < argc; index++) {
        printf("arg.%d=%s\n", index, argv[index]);
    }
    printf("stdin-begin\n");
    fflush(stdout);

    unsigned char buffer[4096];
    for (;;) {
        size_t count = fread(buffer, 1, sizeof(buffer), stdin);
        if (count > 0 && fwrite(buffer, 1, count, stdout) != count) {
            fprintf(stderr, "stdout write failed\n");
            return 4;
        }
        if (count < sizeof(buffer)) {
            if (ferror(stdin)) {
                fprintf(stderr, "stdin read failed\n");
                return 5;
            }
            break;
        }
    }
    printf("\nstdin-end\n");
    fflush(stdout);
    fprintf(stderr, "%s\n", stderr_text);
    fflush(stderr);

    if (sleep_ms > 0 && sleep_milliseconds(sleep_ms) != 0) {
        fprintf(stderr, "sleep failed: %s\n", strerror(errno));
        return 6;
    }
    return (int)exit_code;
}
