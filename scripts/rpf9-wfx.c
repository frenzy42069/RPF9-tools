/* Read-only Double Commander WFX bridge.
 *
 * Build with -DRPF9_WFX_BACKEND='"/absolute/path/rpf9-wfx-backend.py"'.
 * The helper receives one argument per path; it is never run through a shell.
 */
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef RPF9_WFX_BACKEND
#error "Set RPF9_WFX_BACKEND to the absolute path of rpf9-wfx-backend.py"
#endif

/* Definitions follow Double Commander's SDK common.h on non-Windows. */
#pragma pack(push, 1)
typedef struct {
    uint32_t low, high;
} FileTime;

typedef struct {
    uint32_t attributes;
    FileTime creation, access, modified;
    uint32_t size_high, size_low, reserved0, reserved1;
    char name[260], alternate_name[14];
} FindData;
#pragma pack(pop)

typedef int (*ProgressFn)(int, char *, char *, int);
typedef void (*LogFn)(int, int, char *);
typedef int (*RequestFn)(int, int, char *, char *, char *, int);

typedef struct {
    FILE *output;
    pid_t pid;
} Listing;

static int plugin_number;
static LogFn logger;

static void log_message(char *message) {
    if (logger)
        logger(plugin_number, 6, message);
}

static int start_helper(const char *operation, const char *remote,
                        const char *local, int overwrite, pid_t *pid, FILE **output) {
    int fd[2] = {-1, -1};
    if (output && pipe(fd) != 0)
        return -1;
    pid_t child = fork();
    if (child < 0) {
        if (output) {
            close(fd[0]);
            close(fd[1]);
        }
        return -1;
    }
    if (child == 0) {
        if (output) {
            close(fd[0]);
            if (dup2(fd[1], STDOUT_FILENO) < 0)
                _exit(127);
            close(fd[1]);
        }
        if (!local)
            execl(RPF9_WFX_BACKEND, RPF9_WFX_BACKEND, operation, remote, (char *)NULL);
        else if (overwrite)
            execl(RPF9_WFX_BACKEND, RPF9_WFX_BACKEND, operation, remote, local,
                  "--overwrite", (char *)NULL);
        else
            execl(RPF9_WFX_BACKEND, RPF9_WFX_BACKEND, operation, remote, local, (char *)NULL);
        _exit(127);
    }
    *pid = child;
    if (output) {
        close(fd[1]);
        *output = fdopen(fd[0], "r");
        if (!*output) {
            close(fd[0]);
            kill(child, SIGTERM);
            while (waitpid(child, NULL, 0) < 0 && errno == EINTR) {}
            return -1;
        }
    }
    return 0;
}

static int wait_helper(pid_t pid) {
    int status;
    pid_t result;
    do {
        result = waitpid(pid, &status, 0);
    } while (result < 0 && errno == EINTR);
    return result == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

static int read_entry(Listing *listing, FindData *data) {
    char *line = NULL;
    size_t capacity = 0;
    ssize_t length = getline(&line, &capacity, listing->output);
    if (length < 0) {
        free(line);
        return 0;
    }
    while (length && (line[length - 1] == '\n' || line[length - 1] == '\r'))
        line[--length] = '\0';
    char *first = strchr(line, '\t');
    char *second = first ? strchr(first + 1, '\t') : NULL;
    if (!first || !second || first != line + 1 || (line[0] != 'D' && line[0] != 'F')) {
        free(line);
        log_message("RPF9 backend returned an invalid listing entry");
        return 0;
    }
    *second = '\0';
    errno = 0;
    char *end;
    uintmax_t size = strtoumax(first + 1, &end, 10);
    char *name = second + 1;
    size_t name_length = strlen(name);
    if (first[1] < '0' || first[1] > '9' || errno || end != second ||
        size > UINT64_MAX || !name_length ||
        name_length >= sizeof(data->name) || strchr(name, '/') || strchr(name, '\\') ||
        !strcmp(name, ".") || !strcmp(name, "..")) {
        free(line);
        log_message("RPF9 backend returned an invalid listing field");
        return 0;
    }
    memset(data, 0, sizeof(*data));
    data->attributes = line[0] == 'D' ? 16 : 1;
    data->size_high = (uint32_t)(size >> 32);
    data->size_low = (uint32_t)size;
    memcpy(data->name, name, name_length + 1);
    free(line);
    return 1;
}

int FsInit(int number, ProgressFn progress, LogFn log, RequestFn request) {
    (void)progress;
    (void)request;
    plugin_number = number;
    logger = log;
    return 0;
}

void *FsFindFirst(char *path, FindData *data) {
    Listing *listing = calloc(1, sizeof(*listing));
    if (!listing)
        return (void *)(intptr_t)-1;
    if (start_helper("list", path, NULL, 0, &listing->pid, &listing->output) != 0) {
        free(listing);
        log_message("Cannot start RPF9 listing helper");
        return (void *)(intptr_t)-1;
    }
    if (!read_entry(listing, data)) {
        fclose(listing->output);
        if (!wait_helper(listing->pid))
            log_message("RPF9 listing failed");
        free(listing);
        return (void *)(intptr_t)-1;
    }
    return listing;
}

int FsFindNext(void *handle, FindData *data) {
    if (!handle || handle == (void *)(intptr_t)-1)
        return 0;
    return read_entry((Listing *)handle, data);
}

int FsFindClose(void *handle) {
    if (!handle || handle == (void *)(intptr_t)-1)
        return 0;
    Listing *listing = handle;
    fclose(listing->output);
    int success = wait_helper(listing->pid);
    free(listing);
    return success ? 0 : 1;
}

int FsGetFile(char *remote, char *local, int flags, void *remote_info) {
    (void)remote_info;
    if (flags & 6) /* Resume and move need different semantics. */
        return 6;
    if (!(flags & 1) && access(local, F_OK) == 0)
        return 1;
    pid_t child;
    if (start_helper("extract", remote, local, flags & 1, &child, NULL) != 0) {
        log_message("Cannot start RPF9 extraction helper");
        return 3;
    }
    if (!wait_helper(child)) {
        log_message("RPF9 extraction failed");
        return 3;
    }
    return 0;
}

int FsExecuteFile(void *window, char *remote, char *verb) {
    (void)window;
    (void)remote;
    (void)verb;
    return -1;
}

void FsGetDefRootName(char *name, int maxlen) {
    if (maxlen > 0)
        snprintf(name, (size_t)maxlen, "RPF9");
}

int FsGetBackgroundFlags(void) {
    return 0;
}
