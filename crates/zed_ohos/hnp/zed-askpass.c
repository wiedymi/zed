#include <errno.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static int write_all(int fd, const char *buffer, size_t length) {
    while (length > 0) {
        ssize_t written = write(fd, buffer, length);
        if (written < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        buffer += written;
        length -= (size_t)written;
    }
    return 0;
}

int main(int argc, char **argv) {
    const char *socket_path = getenv("ZED_ASKPASS_SOCKET");
    if (socket_path == NULL || socket_path[0] == '\0') {
        fputs("ZED_ASKPASS_SOCKET is not set\n", stderr);
        return 1;
    }

    struct sockaddr_un address = {0};
    address.sun_family = AF_UNIX;
    size_t socket_path_length = strlen(socket_path);
    if (socket_path_length >= sizeof(address.sun_path)) {
        fputs("ZED_ASKPASS_SOCKET is too long\n", stderr);
        return 1;
    }
    memcpy(address.sun_path, socket_path, socket_path_length + 1);

    int socket_fd = socket(AF_UNIX, SOCK_STREAM, 0);
    if (socket_fd < 0) {
        perror("creating askpass socket");
        return 1;
    }
    socklen_t address_length =
        (socklen_t)(offsetof(struct sockaddr_un, sun_path) + socket_path_length + 1);
    if (connect(socket_fd, (struct sockaddr *)&address, address_length) < 0) {
        perror("connecting to askpass socket");
        close(socket_fd);
        return 1;
    }

    for (int index = 1; index < argc; index++) {
        if (index > 1 && write_all(socket_fd, "\0", 1) < 0) {
            perror("writing askpass prompt separator");
            close(socket_fd);
            return 1;
        }
        if (write_all(socket_fd, argv[index], strlen(argv[index])) < 0) {
            perror("writing askpass prompt");
            close(socket_fd);
            return 1;
        }
    }
    if (write_all(socket_fd, "\0", 1) < 0) {
        perror("terminating askpass prompt");
        close(socket_fd);
        return 1;
    }
    if (shutdown(socket_fd, SHUT_WR) < 0) {
        perror("finishing askpass prompt");
        close(socket_fd);
        return 1;
    }

    char response[4096];
    for (;;) {
        ssize_t read_count = read(socket_fd, response, sizeof(response));
        if (read_count == 0) {
            break;
        }
        if (read_count < 0) {
            if (errno == EINTR) {
                continue;
            }
            perror("reading askpass response");
            close(socket_fd);
            return 1;
        }
        if (write_all(STDOUT_FILENO, response, (size_t)read_count) < 0) {
            perror("writing askpass response");
            close(socket_fd);
            return 1;
        }
    }

    close(socket_fd);
    return 0;
}
