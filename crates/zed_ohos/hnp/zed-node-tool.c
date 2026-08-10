#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static const char *program_name(const char *path) {
    const char *slash = strrchr(path, '/');
    return slash == NULL ? path : slash + 1;
}

static int configure_npm_environment(const char *tool) {
    /*
     * HarmonyOS gives ordinary applications a public-looking HOME that they
     * cannot mutate. npm otherwise chooses $HOME/.npm and fails while
     * renaming cache entries. These virtual paths always resolve inside the
     * calling application's own EL2 sandbox.
     */
    if (setenv("HOME", "/data/storage/el2/base/files", 1) < 0 ||
        setenv("npm_config_cache", "/data/storage/el2/base/cache/npm", 1) < 0 ||
        setenv("TMPDIR", "/data/storage/el2/base/cache", 1) < 0) {
        fprintf(stderr, "%s: configuring private npm storage: %s\n", tool, strerror(errno));
        return -1;
    }
    return 0;
}

int main(int argc, char **argv) {
    char executable[PATH_MAX];
    ssize_t executable_length = readlink("/proc/self/exe", executable, sizeof(executable) - 1);
    if (executable_length < 0) {
        fprintf(stderr, "%s: readlink(/proc/self/exe): %s\n", program_name(argv[0]), strerror(errno));
        return 127;
    }
    executable[executable_length] = '\0';

    char *separator = strrchr(executable, '/');
    if (separator == NULL) {
        fprintf(stderr, "%s: executable has no parent directory: %s\n", program_name(argv[0]), executable);
        return 127;
    }
    *separator = '\0';

    const char *tool = strcmp(program_name(argv[0]), "npx") == 0 ? "npx" : "npm";
    if (configure_npm_environment(tool) < 0) {
        return 127;
    }
    char node[PATH_MAX];
    char entrypoint[PATH_MAX];
    int node_length = snprintf(node, sizeof(node), "%s/node", executable);
    int entrypoint_length = snprintf(
        entrypoint,
        sizeof(entrypoint),
        "%s/../lib/node_modules/npm/bin/%s-cli.js",
        executable,
        tool
    );
    if (node_length < 0 || (size_t)node_length >= sizeof(node) ||
        entrypoint_length < 0 || (size_t)entrypoint_length >= sizeof(entrypoint)) {
        fprintf(stderr, "%s: packaged Node path is too long\n", program_name(argv[0]));
        return 127;
    }

    char **node_argv = calloc((size_t)argc + 2, sizeof(char *));
    if (node_argv == NULL) {
        fprintf(stderr, "%s: allocating arguments: %s\n", program_name(argv[0]), strerror(errno));
        return 127;
    }
    node_argv[0] = node;
    node_argv[1] = entrypoint;
    for (int index = 1; index < argc; ++index) {
        node_argv[index + 1] = argv[index];
    }

    execv(node, node_argv);
    fprintf(stderr, "%s: execv(%s): %s\n", program_name(argv[0]), node, strerror(errno));
    free(node_argv);
    return 127;
}
