/* Test double for the Claude CLI. A Rust test appends a config trailer. */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define MAGIC "DASDEVBOT_FAKE_CFG\n"

struct cfg {
    char mode[64];
    char ran[4096];
    char executed[4096];
    char argv_path[4096];
    char cwd_path[4096];
    char mode_file[4096];
    char prompt_copy[4096];
    char pidfile[4096];
    char survived[4096];
    char home_record[4096];
    char found[4096];
    char leak[4096];
};

static void set_key(struct cfg *cfg, const char *key, const char *value) {
    char *dest = NULL;
    size_t cap = 0;
    if (strcmp(key, "mode") == 0) {
        dest = cfg->mode;
        cap = sizeof cfg->mode;
    } else if (strcmp(key, "ran") == 0) {
        dest = cfg->ran;
        cap = sizeof cfg->ran;
    } else if (strcmp(key, "executed") == 0) {
        dest = cfg->executed;
        cap = sizeof cfg->executed;
    } else if (strcmp(key, "argv") == 0) {
        dest = cfg->argv_path;
        cap = sizeof cfg->argv_path;
    } else if (strcmp(key, "cwd") == 0) {
        dest = cfg->cwd_path;
        cap = sizeof cfg->cwd_path;
    } else if (strcmp(key, "mode_file") == 0) {
        dest = cfg->mode_file;
        cap = sizeof cfg->mode_file;
    } else if (strcmp(key, "prompt_copy") == 0) {
        dest = cfg->prompt_copy;
        cap = sizeof cfg->prompt_copy;
    } else if (strcmp(key, "pidfile") == 0) {
        dest = cfg->pidfile;
        cap = sizeof cfg->pidfile;
    } else if (strcmp(key, "survived") == 0) {
        dest = cfg->survived;
        cap = sizeof cfg->survived;
    } else if (strcmp(key, "home_record") == 0) {
        dest = cfg->home_record;
        cap = sizeof cfg->home_record;
    } else if (strcmp(key, "found") == 0) {
        dest = cfg->found;
        cap = sizeof cfg->found;
    } else if (strcmp(key, "leak") == 0) {
        dest = cfg->leak;
        cap = sizeof cfg->leak;
    }
    if (dest == NULL || cap == 0) {
        return;
    }
    snprintf(dest, cap, "%s", value);
}

static int load_cfg(struct cfg *cfg) {
    memset(cfg, 0, sizeof *cfg);
    int fd = open("/proc/self/exe", O_RDONLY);
    if (fd < 0) {
        return -1;
    }
    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size <= 0 || st.st_size > 32 * 1024 * 1024) {
        close(fd);
        return -1;
    }
    char *buf = malloc((size_t)st.st_size + 1);
    if (buf == NULL) {
        close(fd);
        return -1;
    }
    size_t got = 0;
    while (got < (size_t)st.st_size) {
        ssize_t n = read(fd, buf + got, (size_t)st.st_size - got);
        if (n < 0) {
            free(buf);
            close(fd);
            return -1;
        }
        if (n == 0) {
            break;
        }
        got += (size_t)n;
    }
    close(fd);
    buf[got] = 0;
    char *at = NULL;
    for (char *cursor = buf; cursor + sizeof MAGIC - 1 <= buf + got; cursor++) {
        if (memcmp(cursor, MAGIC, sizeof MAGIC - 1) == 0) {
            at = cursor;
        }
    }
    if (at == NULL) {
        free(buf);
        return -1;
    }
    at += sizeof MAGIC - 1;
    char *line = at;
    while (*line != 0) {
        char *nl = strchr(line, '\n');
        if (nl != NULL) {
            *nl = 0;
        }
        char *eq = strchr(line, '=');
        if (eq != NULL) {
            *eq = 0;
            set_key(cfg, line, eq + 1);
        }
        if (nl == NULL) {
            break;
        }
        line = nl + 1;
    }
    free(buf);
    return cfg->mode[0] == 0 ? -1 : 0;
}

static void touch_path(const char *path) {
    if (path == NULL || path[0] == 0) {
        return;
    }
    int fd = open(path, O_CREAT | O_WRONLY, 0644);
    if (fd >= 0) {
        close(fd);
    }
}

static void drain_stdin(void) {
    char buf[4096];
    while (read(STDIN_FILENO, buf, sizeof buf) > 0) {
    }
}

static int link_is_snapshot(const char *link) {
    return strstr(link, "memfd:dasdevbot-claude") != NULL;
}

static int self_has_snapshot(void) {
    DIR *dir = opendir("/proc/self/fd");
    if (dir == NULL) {
        return 0;
    }
    int found = 0;
    int dirfd_no = dirfd(dir);
    struct dirent *ent;
    while ((ent = readdir(dir)) != NULL) {
        if (ent->d_name[0] == '.') {
            continue;
        }
        int fd = atoi(ent->d_name);
        if (fd == dirfd_no) {
            continue;
        }
        char path[64];
        char link[256];
        snprintf(path, sizeof path, "/proc/self/fd/%d", fd);
        ssize_t n = readlink(path, link, sizeof link - 1);
        if (n < 0) {
            continue;
        }
        link[n] = 0;
        if (link_is_snapshot(link)) {
            found = 1;
            break;
        }
    }
    closedir(dir);
    return found;
}

static void note_snapshot(const char *leak) {
    if (leak == NULL || leak[0] == 0) {
        return;
    }
    int leaked = self_has_snapshot();
    pid_t pid = fork();
    if (pid == 0) {
        _exit(self_has_snapshot() ? 3 : 0);
    }
    if (pid > 0) {
        int status = 0;
        if (waitpid(pid, &status, 0) == pid && WIFEXITED(status) && WEXITSTATUS(status) == 3) {
            leaked = 1;
        }
    }
    if (leaked) {
        int fd = open(leak, O_CREAT | O_WRONLY | O_TRUNC, 0644);
        if (fd >= 0) {
            const char text[] = "leaked\n";
            if (write(fd, text, sizeof text - 1) < 0) {
                /* The test only checks that this file exists. */
            }
            close(fd);
        }
    }
}

static int env_is(const char *key, const char *want) {
    const char *value = getenv(key);
    return value != NULL && strcmp(value, want) == 0;
}

static int secret_set(const char *key) {
    const char *value = getenv(key);
    return value != NULL && value[0] != 0;
}

static const char *prompt_arg(int argc, char **argv) {
    for (int i = 1; i < argc - 1; i++) {
        if (strcmp(argv[i], "--system-prompt-file") == 0) {
            return argv[i + 1];
        }
    }
    return "";
}

static const char *flag_value(int argc, char **argv, const char *flag) {
    for (int i = 1; i < argc - 1; i++) {
        if (strcmp(argv[i], flag) == 0) {
            return argv[i + 1];
        }
    }
    return "";
}

static void copy_file(const char *from, const char *to) {
    FILE *in = fopen(from, "r");
    if (in == NULL) {
        return;
    }
    FILE *out = fopen(to, "w");
    if (out == NULL) {
        fclose(in);
        return;
    }
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, in)) > 0) {
        fwrite(buf, 1, n, out);
    }
    fclose(in);
    fclose(out);
}

static void record_argv(const char *path, int argc, char **argv) {
    FILE *file = fopen(path, "w");
    if (file == NULL) {
        return;
    }
    for (int i = 1; i < argc; i++) {
        fputs(argv[i], file);
        fputc('\0', file);
    }
    fclose(file);
}

static void record_cwd_mode(const struct cfg *cfg) {
    char cwd[4096];
    if (getcwd(cwd, sizeof cwd) == NULL) {
        return;
    }
    if (cfg->cwd_path[0] != 0) {
        FILE *file = fopen(cfg->cwd_path, "w");
        if (file != NULL) {
            fprintf(file, "%s\n", cwd);
            fclose(file);
        }
    }
    if (cfg->mode_file[0] != 0) {
        struct stat st;
        if (stat(".", &st) == 0) {
            FILE *file = fopen(cfg->mode_file, "w");
            if (file != NULL) {
                fprintf(file, "%o %u\n", st.st_mode & 0777, (unsigned)st.st_uid);
                fclose(file);
            }
        }
    }
}

static void walk_claude_md(const char *found) {
    char dir[4096];
    if (getcwd(dir, sizeof dir) == NULL) {
        return;
    }
    while (dir[0] != 0 && strcmp(dir, "/") != 0) {
        char path[8192];
        snprintf(path, sizeof path, "%s/CLAUDE.md", dir);
        if (access(path, F_OK) == 0) {
            touch_path(found);
        }
        char *slash = strrchr(dir, '/');
        if (slash == NULL) {
            break;
        }
        if (slash == dir) {
            dir[1] = 0;
            break;
        }
        *slash = 0;
    }
}

static int has_source(const char *sources, const char *name) {
    if (sources == NULL || sources[0] == 0) {
        return 1;
    }
    char padded[1024];
    char want[64];
    snprintf(padded, sizeof padded, ",%s,", sources);
    snprintf(want, sizeof want, ",%s,", name);
    return strstr(padded, want) != NULL;
}

static int has_flag(int argc, char **argv, const char *flag) {
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], flag) == 0) {
            return 1;
        }
    }
    return 0;
}

/* Runs every `touch PATH` command found in a settings file. */
static void run_touches(const char *settings_path, const char *needle) {
    FILE *file = fopen(settings_path, "r");
    if (file == NULL) {
        return;
    }
    char buf[8192];
    size_t n = fread(buf, 1, sizeof buf - 1, file);
    fclose(file);
    buf[n] = 0;
    char *cursor = buf;
    while ((cursor = strstr(cursor, needle)) != NULL) {
        char *touch = strstr(cursor, "touch ");
        if (touch == NULL) {
            return;
        }
        touch += 6;
        char marker[4096];
        size_t i = 0;
        while (touch[i] != 0 && touch[i] != '"' && touch[i] != ' ' && touch[i] != ';' &&
               i + 1 < sizeof marker) {
            marker[i] = touch[i];
            i++;
        }
        marker[i] = 0;
        touch_path(marker);
        cursor = touch;
    }
}

/* Project root: the nearest ancestor of the cwd that holds .git, else the cwd. */
static void project_root(char *out, size_t cap) {
    char dir[4096];
    if (getcwd(dir, sizeof dir) == NULL) {
        out[0] = 0;
        return;
    }
    snprintf(out, cap, "%s", dir);
    while (dir[0] != 0 && strcmp(dir, "/") != 0) {
        char git[8192];
        snprintf(git, sizeof git, "%s/.git", dir);
        if (access(git, F_OK) == 0) {
            snprintf(out, cap, "%s", dir);
            return;
        }
        char *slash = strrchr(dir, '/');
        if (slash == NULL || slash == dir) {
            return;
        }
        *slash = 0;
    }
}

/*
 * Models what Claude Code 2.1.285 did offline (unshare -rn) with planted settings:
 * - User settings come from $CLAUDE_CONFIG_DIR, else $HOME/.claude.
 * - Project settings come from the project root's .claude/settings.json.
 * - Hooks run unless --settings sets disableAllHooks, --safe-mode or --restricted is
 *   passed, or --setting-sources omits that source.
 * - A project or user apiKeyHelper runs unless --restricted is passed or the source is
 *   omitted. disableAllHooks and --safe-mode did not stop it.
 */
static void hostile(const struct cfg *cfg, int argc, char **argv) {
    const char *home = getenv("HOME");
    const char *config = getenv("CLAUDE_CONFIG_DIR");
    if (cfg->home_record[0] != 0) {
        FILE *file = fopen(cfg->home_record, "w");
        if (file != NULL) {
            fprintf(file, "%s\n%s\n", home ? home : "", config ? config : "");
            fclose(file);
        }
    }
    const char *sources = flag_value(argc, argv, "--setting-sources");
    const char *settings = flag_value(argc, argv, "--settings");
    int restricted = has_flag(argc, argv, "--restricted");
    int hooks_off = restricted || has_flag(argc, argv, "--safe-mode") ||
                    (settings != NULL && strstr(settings, "disableAllHooks") != NULL);
    char user_settings[8192] = {0};
    if (config != NULL && config[0] != 0) {
        snprintf(user_settings, sizeof user_settings, "%s/settings.json", config);
    } else if (home != NULL) {
        snprintf(user_settings, sizeof user_settings, "%s/.claude/settings.json", home);
    }
    char root[4096];
    char project_settings[8192];
    project_root(root, sizeof root);
    snprintf(project_settings, sizeof project_settings, "%s/.claude/settings.json", root);
    if (!restricted && has_source(sources, "user") && user_settings[0] != 0) {
        if (!hooks_off) {
            run_touches(user_settings, "\"hooks\"");
        }
        run_touches(user_settings, "\"apiKeyHelper\"");
    }
    if (!restricted && has_source(sources, "project")) {
        if (!hooks_off) {
            run_touches(project_settings, "\"hooks\"");
        }
        run_touches(project_settings, "\"apiKeyHelper\"");
    }
}

static void grandchild(const struct cfg *cfg) {
    pid_t pid = fork();
    if (pid == 0) {
        FILE *file = fopen(cfg->pidfile, "w");
        if (file != NULL) {
            fprintf(file, "%d\n", (int)getpid());
            fclose(file);
        }
        sleep(30);
        touch_path(cfg->survived);
        _exit(0);
    }
    for (int i = 0; i < 100; i++) {
        struct stat st;
        if (stat(cfg->pidfile, &st) == 0 && st.st_size > 0) {
            break;
        }
        usleep(50 * 1000);
    }
    puts(
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\"name\":"
        "\"Bash\",\"input\":{\"command\":\"SENTINEL_TOOL_BODY\"}}]}}");
    fflush(stdout);
    if (pid > 0) {
        waitpid(pid, NULL, 0);
    }
}

static void print_ok(void) {
    puts("{\"type\":\"result\",\"result\":\"ok\",\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}");
}

int main(int argc, char **argv) {
    struct cfg cfg;
    if (load_cfg(&cfg) != 0) {
        fprintf(stderr, "fake claude config missing\n");
        return 9;
    }
    note_snapshot(cfg.leak);
    if (argc > 1 && strcmp(argv[1], "--version") == 0) {
        if (strcmp(cfg.mode, "bad_version") == 0) {
            puts("9.9.9");
        } else {
            puts("2.1.285 (Claude Code)");
        }
        return 0;
    }
    if (strcmp(cfg.mode, "bad_version") == 0) {
        touch_path(cfg.ran);
        return 0;
    }
    drain_stdin();
    if (strcmp(cfg.mode, "argv") == 0) {
        if (!env_is("DISABLE_AUTOUPDATER", "1") || !env_is("DISABLE_UPDATES", "1") ||
            !env_is("PATH", "/usr/bin:/bin") || getenv("CLAUDE_CONFIG_DIR") == NULL ||
            strstr(getenv("CLAUDE_CONFIG_DIR"), "/claude-config") == NULL) {
            touch_path(cfg.executed);
            return 2;
        }
        if (secret_set("OLLAMA_API_KEY") || secret_set("ANTHROPIC_API_KEY") ||
            secret_set("DASDEVBOT_TOKEN") || secret_set("CLAUDE_CODE_OAUTH_TOKEN")) {
            touch_path(cfg.executed);
            return 2;
        }
        record_argv(cfg.argv_path, argc, argv);
        record_cwd_mode(&cfg);
        copy_file(prompt_arg(argc, argv), cfg.prompt_copy);
        puts(
            "{\"type\":\"rate_limit_event\",\"rate_limit_info\":{\"status\":\"allowed\","
            "\"resetsAt\":1700000000,\"utilization\":0.1}}");
        puts(
            "{\"type\":\"result\",\"result\":\"ok\",\"usage\":{\"input_tokens\":2,\"output_"
            "tokens\":1}}");
        return 0;
    }
    if (strcmp(cfg.mode, "grandchild") == 0) {
        grandchild(&cfg);
        return 0;
    }
    if (strcmp(cfg.mode, "hostile") == 0) {
        hostile(&cfg, argc, argv);
        print_ok();
        return 0;
    }
    if (strcmp(cfg.mode, "cwd") == 0) {
        record_cwd_mode(&cfg);
        copy_file(prompt_arg(argc, argv), cfg.prompt_copy);
        walk_claude_md(cfg.found);
        print_ok();
        return 0;
    }
    if (strcmp(cfg.mode, "ok") == 0) {
        print_ok();
        return 0;
    }
    fprintf(stderr, "fake claude unknown mode\n");
    return 9;
}
