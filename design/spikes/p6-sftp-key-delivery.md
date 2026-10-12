# Spike P6: SFTP key delivery

- Status: done
- Date: 2026-10-12
- Requirements: FR-AG-04, FR-AG-06, FR-RP-01
- Decision record: none yet. The Recommendation proposes a new ADR and changes to ADR-0021.

## Question

The control plane sends an SFTP private key inside a job (ADR-0003). The agent holds the key in memory. How can the agent give the key to restic 0.18.1 and 0.19.x without a file on the agent? The answer must work on Linux (agent runs as root) and on Windows (agent runs as LocalSystem).

The answer must also cover the host key check. The job can carry the public host key of the SFTP server. The agent must check it without a `known_hosts` file.

The agent builds all restic arguments itself (ADR-0021). Secrets must not go on a command line (FR-AG-06).

## Options

| ID | Option | Short description |
|---|---|---|
| (a) | Private ssh-agent for each job | The agent starts `ssh-agent -D -a <socket>`. It loads the key with `ssh-add -` from a pipe. It gives `SSH_AUTH_SOCK` to restic. The restic `ssh` child inherits it. |
| (b) | restic `rclone:` backend with rclone `key_pem` | The agent sets `RCLONE_CONFIG_<NAME>_*` environment variables. rclone connects with its own Go SSH client. No `ssh` program runs. |
| (c) | Anonymous in-memory file | Linux `memfd_create`. The agent passes `-i /proc/<agent pid>/fd/<N>` in `-o sftp.args`. |
| (d) | Temporary file with a strict ACL | The agent writes the key to a file, passes `-i <file>` and deletes the file after the job. This is the fallback. It violates FR-AG-04. |
| (e) | ssh-agent inside the Rust agent | The agent serves the ssh-agent protocol on a Unix socket or a named pipe with a random name. Not tested. |

## Comparison table

"Tested" means that the spike ran the option on the Linux host. "Source" means that the statement comes from source code or documentation only.

| Option | Linux | Windows | Key on command line or disk | Extra to ship | Fits ADR-0021 | Fits FR-AG-04 and FR-AG-06 |
|---|---|---|---|---|---|---|
| (a) ssh-agent for each job | Yes (tested) | No. The Windows agent is a service with a fixed pipe name. It stores keys in the registry. | No command line. Key in RAM of `ssh-agent`. The socket file holds no secret. | OpenSSH client tools | Yes. The agent builds `sftp.args` from constants. The host key needs a path. | Yes on Linux. The key goes through a pipe. |
| (b) rclone `key_pem` in environment | Yes (tested) | Yes (source) | No command line. Key in the environment of restic and rclone. No file when `SHELL_TYPE=none`. | rclone 1.75.0 or newer. The Linux amd64 binary has 85.7 MB. | Yes. The agent fixes the names and the program. ADR-0021 needs more value rules. | Yes. The key goes through environment variables. |
| (c) `memfd` and `ssh -i` | Yes (tested). Only with `/proc/<agent pid>/fd/<N>`. | No. No equivalent. | No command line. No disk. | OpenSSH client tools | Yes. The agent builds `sftp.args`. | FR-AG-04: yes. FR-AG-06: the wording needs a change. |
| (d) temporary file | Yes (source) | Yes (source) | Key on disk | OpenSSH client | Yes | No. It violates FR-AG-04. |
| (e) own ssh-agent | Not tested | Not tested | No command line. No disk. | OpenSSH client. More agent code. | Yes | Yes (design) |

Host key check without a file:

| Option | Method | Result |
|---|---|---|
| (a), (c), (d) | `ssh` reads `UserKnownHostsFile` from a path. On Linux the path can be a `memfd`. | Works on Linux (tested). Windows needs a file. |
| (b) | rclone option `host_keys`, with the entry `<algorithm> <base64>` | Works (tested). rclone checks the key. No file. |

## Findings

### How restic starts the program

1. restic starts `ssh` with `exec.Command(program, args...)`. It does not set `cmd.Env`. The child process gets the full environment of restic [1].
2. restic builds the command as `ssh <host> [-p <port>] [-l <user>] <sftp.args> -s sftp`. If `sftp.command` is set, restic runs that command instead [1].
3. `sftp.command` and `sftp.args` exclude each other. restic returns the error "cannot specify both sftp.command and sftp.args options". The restic documentation says that `sftp.args` is "ignored" [1][5]. The source is correct.
4. stdin and stdout of the child process carry the SFTP stream. stderr is a pipe. A key cannot come from `-i /dev/stdin` [1].
5. The `rclone:` backend uses the same pattern. It runs `rclone.program`, then `rclone.args`, then the text after `rclone:`. It also does not set `cmd.Env` [3][4][5].
6. restic has no environment variable for extended options. The agent must pass `-o sftp.args=...` or `-o rclone.program=...` on the restic command line. These values hold no secret.
7. A host name that starts with `-` can look like an `ssh` option. The test used `sftp:u@-oProxyCommand=touch <file>:/x`. `ssh` printed its usage text and ran nothing. The agent must still reject such host names.

### Option (a): private ssh-agent

1. `ssh-add -` reads the private key from stdin. The source of `ssh-add.c` handles the file name `-` as `STDIN_FILENO` [8].
2. `ssh-agent -D -a <path>` runs in the foreground on a given socket. It turns off tracing and core dumps for itself [9][10].
3. The test started `ssh-agent`, loaded the key from a shell pipe and ran restic with `SSH_AUTH_SOCK`. The repository opened. The authorized key of the test server was only the loaded key. This proves that the agent supplied the key. The test used restic 0.18.1 and 0.19.1 [24].
4. The socket path is a file name. The agent must create a directory with mode 0700 for it. The socket path has a limit of 107 bytes.
5. If the Rust agent stops, the `ssh-agent` child can stay. The agent must set a parent-death signal and must remove the directory at start. This is not tested.
6. On Windows, option (a) fails. The Windows `ssh-agent.exe` is a service. The source defines one pipe name, `\\.\pipe\openssh-ssh-agent`. The debug mode (`-d`) uses the same pipe [12][13].
7. The Windows `ssh-agent` writes each added key to the registry. The add function in `keyagent-request.c` calls `RegCreateKeyExW` and `RegSetValueExW`. DPAPI encrypts the key, but the key stays after a reboot [14].
8. The Win32-OpenSSH team calls this behavior "by design". Issue 1487 is open (last update 2026-10-03). A plan from 2021 describes a mode without persistence. I found no such mode in the source that I read [11].
9. A persistent key store violates FR-AG-04. The fixed pipe also conflicts with any user agent on the same host.

### Option (b): rclone with `key_pem`

1. rclone reads remote options from `RCLONE_CONFIG_<NAME>_<OPTION>`. The variable `..._TYPE` is required [21]. The test used `RCLONE_CONFIG_RH_TYPE=sftp` and `-r rclone:rh:<path>`.
2. rclone does not accept real line breaks in `key_pem`. It runs `strconv.Unquote` on the value inside double quotes [20]. The test with real line breaks failed with "pem key not formatted properly".
3. The agent must replace each line break with the two characters `\n`. The value must not contain a double quote or a backslash. Base64 and the PEM header have none of these.
4. The key is in the environment of the restic child process and the rclone process. The `/proc/<pid>/environ` file of rclone has mode 0400. The rclone command line shows only `serve restic --stdio ... rh:<path>` [24].
5. rclone writes a file `rclone.conf` in the user home directory. The file holds the detected `shell_type` and two command names. It holds no key. The test confirmed that the key is not in the file [24].
6. With `RCLONE_CONFIG_RH_SHELL_TYPE=none`, rclone writes no file. This was tested with the home directory of the test user. If rclone cannot write the file, it retries 10 times. The test showed a delay of 17 seconds [24].
7. By default, rclone does not check the host key. It prints "No host key validation is being performed" and connects. The test confirmed this [19][24]. The agent must refuse a job without a host key.
8. The option `host_keys` takes a comma-separated list of `<algorithm> <base64>`. rclone 1.75.0 added it, with host key pinning [19][22]. The test with the correct key passed. The test with a wrong key failed with "host key mismatch".
9. `host_keys` has three limits. It cannot hold an `@cert-authority` entry. It holds at most 16 entries. The entry has the algorithm and the base64 key, without the host name [19][20].
10. rclone uses its own Go SSH client. It needs no `ssh` program. This removes the Windows dependency on the optional OpenSSH client feature [18][19].
11. rclone options `ssh`, `key_file`, `known_hosts_file` and `md5sum_command` read files or run programs. The agent must never set them. With a fixed list of names, job data cannot set them [19].
12. The Linux amd64 binary of rclone v1.75.2 (released 2026-10-09) has 85,659,810 bytes [23].
13. I did not measure throughput. rclone adds one process and one protocol layer between restic and the SFTP server.

### Option (c): anonymous in-memory file

1. `ssh` calls `closefrom(STDERR_FILENO + 1)` at start [7]. A file descriptor that the agent passes to restic does not reach `ssh` as a usable descriptor. The test with `-i /proc/self/fd/3` failed: "Identity file ... not accessible".
2. The workaround is a close-on-exec `memfd` that stays in the agent. The agent passes `-i /proc/<agent pid>/fd/<N>`. The `ssh` child opens the path with the same user identity. The test passed with restic 0.18.1 and 0.19.1 [24].
3. The agent must call `fchmod(fd, 0600)`. `ssh` checks the file mode of the key.
4. The host key also works in a second `memfd` with `-o UserKnownHostsFile=/proc/<agent pid>/fd/<M>`. A wrong host key gave "Host key verification failed" (tested).
5. This option depends on read access to `/proc/<pid>/fd`. The access checks of the kernel can block it. Examples are a non-dumpable agent process, `hidepid` and a missing `CAP_SYS_PTRACE` in a container. I did not test these cases.
6. FR-AG-06 allows only environment variables and pipes. A `memfd` path is neither.
7. Windows has no `memfd` and no `/proc`. `ssh.exe` checks the owner and the ACL of a key file with `check_secure_file_permission` [16]. A named pipe path is not a supported key file. This is not tested.

### Option (d): temporary file

1. The key is on disk, even for a short time. A crash or a power loss leaves the file. This violates FR-AG-04.
2. If the team accepts option (d) as a fallback, these rules apply. The file is in a directory that only root or LocalSystem can read. The agent deletes it after the job. The agent also deletes old files at start. On Linux the directory is on `tmpfs`.
3. On Windows, `ssh.exe` accepts a key file that SYSTEM or the Administrators group owns, with no ACL entry for other users [16].

### Option (e): ssh-agent in the Rust agent

1. The ssh-agent protocol is small. The agent needs the identity list and the sign request. The agent needs signing code for ed25519, ECDSA and RSA keys.
2. The Windows `ssh.exe` source opens the path in `SSH_AUTH_SOCK` with `CreateFileW`. It also sets `SSH_AUTH_SOCK` from `IdentityAgent` [15]. A random pipe name can work in theory. Issue 1987 from 2022 says that the pipe name is fixed. The current source disagrees [17].
3. This option keeps the fast native restic SFTP code on both systems. It adds security-sensitive code to the agent. It needs a real Windows host for tests.

## Recommendation

1. Use option (b) for SFTP repositories on Windows and on Linux. It is the only tested path with no file, no command-line secret and no `ssh` program. One code path serves both systems.
2. The agent must follow these rules:
   - Build the names `RCLONE_CONFIG_RH_TYPE`, `..._HOST`, `..._PORT`, `..._USER`, `..._KEY_PEM`, `..._HOST_KEYS` and `..._SHELL_TYPE` from constants.
   - Put job data only in the values of these variables.
   - Set `SHELL_TYPE=none`.
   - Refuse a job that has no host key.
   - Pass `--repo=rclone:rh:<path>` and `-o rclone.program=<agent-owned path>`. Do not change `rclone.args`.
3. Ship rclone 1.75.0 or newer with the agent package. The agent must check the SHA-256 of the binary before it starts rclone.
4. Do not implement option (d).
5. Before 1.0, measure throughput of rclone against native restic SFTP. If rclone is too slow, build option (e) for both systems. If the team wants a Linux path with native `ssh`, use option (a) for the key and a `memfd` for the host key.
6. Write ADR-0022 for the decision. It introduces a third-party program that runs as root or LocalSystem.

## Effect on requirements

### FR-AG-04

Add this sentence: "The agent never writes a repository private key or a host key list to a file. rclone must not write a configuration file." The agent package contains the rclone program. This is a program file, not stored data. The list of stored items does not change.

### FR-AG-06

No change for option (b). The environment variable is an allowed channel. If the team adds option (c), the text must also allow an anonymous in-memory file. The agent must pass the secret only in the `env` of the child process. It must not call `setenv` in its own process.

### FR-RP-01

- SFTP has priority M. The M milestone therefore needs the rclone program on each agent. The row "rclone" stays at 1.0 for user-defined rclone remotes.
- An SFTP repository needs these fields: host, port, user, path, private key and a list of public host keys. The list is not optional.
- The private key must not have a passphrase. The control plane must reject an encrypted key. Passphrase support is not tested.
- The list of host keys must not hold certificate entries.

### ADR-0021

1. Clarify the last bullet. The rule covers `-o rclone.program`, `-o rclone.args`, `-o sftp.command` and `-o sftp.args` when the value comes from job data. The agent can set `rclone.program` from an agent-owned path.
2. Replace the SFTP key rule. The key must match PEM characters only: letters, digits, `+`, `/`, `=`, `-`, space and line breaks. It must reject `"` and `\`. Reason: rclone runs `strconv.Unquote` on the value.
3. Add formats for the host name, the port and the user. The host name has letters, digits, `.` and `-`, with no leading `-`. A bracketed IPv6 address is also valid. The port is 1 to 65535. The user has no NUL and no line break.
4. Add a format for each host key entry: an algorithm from a short list, then one base64 string.
5. State that the agent builds the names of all `RCLONE_CONFIG_RH_*` variables. The agent never sets the options `ssh`, `key_file`, `known_hosts_file` or any `*_command`.

## Sources

Date opened: 2026-10-12 for all sources.

1. restic v0.19.1, `internal/backend/sftp/sftp.go`: https://raw.githubusercontent.com/restic/restic/v0.19.1/internal/backend/sftp/sftp.go
2. restic v0.19.1, `internal/backend/sftp/config.go`: https://raw.githubusercontent.com/restic/restic/v0.19.1/internal/backend/sftp/config.go
3. restic v0.19.1, `internal/backend/rclone/backend.go`: https://raw.githubusercontent.com/restic/restic/v0.19.1/internal/backend/rclone/backend.go
4. restic v0.19.1, `internal/backend/rclone/config.go`: https://raw.githubusercontent.com/restic/restic/v0.19.1/internal/backend/rclone/config.go
5. restic v0.19.1, `doc/030_preparing_a_new_repo.rst` (SFTP and rclone): https://raw.githubusercontent.com/restic/restic/v0.19.1/doc/030_preparing_a_new_repo.rst
6. restic release v0.19.1, published 2026-07-05: https://api.github.com/repos/restic/restic/releases/latest
7. OpenSSH V_10_2_P1, `ssh.c` (`closefrom`): https://raw.githubusercontent.com/openssh/openssh-portable/V_10_2_P1/ssh.c
8. OpenSSH V_10_2_P1, `ssh-add.c` (`-` reads stdin): https://raw.githubusercontent.com/openssh/openssh-portable/V_10_2_P1/ssh-add.c
9. OpenSSH V_10_2_P1, `ssh-agent.c` (`platform_disable_tracing`, `RLIMIT_CORE`): https://raw.githubusercontent.com/openssh/openssh-portable/V_10_2_P1/ssh-agent.c
10. OpenSSH manual pages `ssh-agent(1)`, `ssh-add(1)`, `ssh_config(5)`. I read the local pages of OpenSSH 10.2p1: https://man.openbsd.org/ssh-agent and https://man.openbsd.org/ssh_config
11. Win32-OpenSSH issue 1487, "ssh-agent: permanent loaded ssh-key storage" (open, last update 2026-10-03): https://github.com/PowerShell/Win32-OpenSSH/issues/1487
12. Win32-OpenSSH source, `contrib/win32/win32compat/ssh-agent/agent.c` (`AGENT_PIPE_ID`), branch `latestw_all`, commit e302fe1ed1 of 2026-10-08: https://raw.githubusercontent.com/PowerShell/openssh-portable/latestw_all/contrib/win32/win32compat/ssh-agent/agent.c
13. Win32-OpenSSH source, `ssh-agent/agent-main.c` (service and debug mode), same branch: https://raw.githubusercontent.com/PowerShell/openssh-portable/latestw_all/contrib/win32/win32compat/ssh-agent/agent-main.c
14. Win32-OpenSSH source, `ssh-agent/keyagent-request.c` (registry writes), same branch: https://raw.githubusercontent.com/PowerShell/openssh-portable/latestw_all/contrib/win32/win32compat/ssh-agent/keyagent-request.c
15. Win32-OpenSSH source, `fileio.c` (`fileio_connect`), `ssh.c` and `authfd.c` (`SSH_AUTH_SOCK`, `IdentityAgent`), same branch: https://raw.githubusercontent.com/PowerShell/openssh-portable/latestw_all/contrib/win32/win32compat/fileio.c
16. Win32-OpenSSH source, `w32-sshfileperm.c` (`check_secure_file_permission`), same branch: https://raw.githubusercontent.com/PowerShell/openssh-portable/latestw_all/contrib/win32/win32compat/w32-sshfileperm.c
17. Win32-OpenSSH issue 1987, "Support for multiple ssh agents" (open, 2022): https://github.com/PowerShell/Win32-OpenSSH/issues/1987
18. Microsoft Learn, "Get started with OpenSSH for Windows" (OpenSSH Client as an optional feature): https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_install_firstuse
19. rclone documentation, SFTP backend (`key_pem`, `host_keys`, `known_hosts_file`, `ssh`): https://raw.githubusercontent.com/rclone/rclone/master/docs/content/sftp.md
20. rclone source, `backend/sftp/sftp.go` (`strconv.Unquote` of `KeyPem`, `host_keys` parser, 16-entry cap): https://raw.githubusercontent.com/rclone/rclone/master/backend/sftp/sftp.go
21. rclone documentation, "Environment Variables" (`RCLONE_CONFIG_<NAME>_<OPTION>`): https://raw.githubusercontent.com/rclone/rclone/master/docs/content/docs.md
22. rclone changelog, v1.75.0 of 2026-07-31 (`--sftp-pin-host-key`): https://raw.githubusercontent.com/rclone/rclone/master/docs/content/changelog.md
23. rclone v1.75.2, released 2026-10-09: https://downloads.rclone.org/v1.75.2/rclone-v1.75.2-linux-amd64.zip
24. Local tests on a Linux host: OpenSSH 10.2p1, restic 0.18.1 and 0.19.1, rclone v1.75.2, a user-level `sshd` on 127.0.0.1. Not published. Scripts in `/tmp/opencode/p6/lab` (`test_a.sh`, `test_b.sh`, `test_b2.sh`, `test_c2.py`, `test_ps.sh`).

## Not verified

- No Windows host was available. All statements about Windows come from source code and documentation. This includes `ssh.exe` with a custom pipe, `-F NUL`, the file ACL check, rclone with environment configuration and `RCLONE_CONFIG` on Windows.
- The Windows source is the branch `latestw_all` of 2026-10-08. The `ssh.exe` and `ssh-agent.exe` that ship with Windows can differ.
- I read only part of `keyagent-request.c`. I did not prove that the interactive mode of the Windows `ssh-agent` always persists keys.
- I did not measure throughput of rclone against native restic SFTP.
- I tested only ed25519 keys. RSA and ECDSA keys with rclone are not tested.
- Passphrase-protected keys are not tested in any option.
- I did not test the access to `/proc/<pid>/fd` for a non-dumpable agent, with `hidepid`, in a container without `CAP_SYS_PTRACE` or under SELinux and AppArmor.
- I did not check whether key memory can go to swap. I did not check core dumps of restic, rclone and `ssh`. No option uses `mlock`.
- I did not check the rclone license, the signing and the update channel for the package.
- Option (e) is not tested. I did not check the available Rust crates.
- I did not test restic 0.19.0. The source check used v0.19.1.
- Option (d) on Linux and on Windows is not tested. It is a fallback that the spike rejects.
