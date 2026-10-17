---
title: Agent logs
description: Where the agent writes its log on Linux and Windows, and how to read it.
---

A command that you start in a console, for example `register` or `status`, writes its log to the standard error.

On Linux the service writes its log to journald. Read it with `journalctl -u restichub-agent`.

On Windows the service writes its log to two locations:

- The directory `%ProgramData%\ResticHub\Agent-logs` holds the full log. Only SYSTEM and Administrators can read it. The files have the names `agent-000001.log`, `agent-000002.log` and so on. The file with the highest number is the current file. When the current file is 10 MB, the service starts the next file. The service keeps 5 files and deletes the oldest file. The directory is not in the identity directory, so `register --force` keeps the log.
- The `ResticHub` event log holds each warning and each error, with the source `ResticHub Agent`. Only SYSTEM, Administrators and the members of Event Log Readers can read this log. Only SYSTEM and Administrators can write to it. The maximum size of the log is 20 MB. When the log is full, Windows overwrites the oldest events. The service control manager writes the start and the stop of the service to the System log.

The service writes the same warning or error to the event log one time in 60 seconds at most. The next copy tells how many identical events it did not write. The log files hold each event.

A line break or a control character in a value changes to an escape sequence, for example `\n`. Thus each event is one line in the log file.

To read the events, open Event Viewer and go to "Applications and Services Logs" > "ResticHub". In PowerShell, use this command:

```powershell
Get-WinEvent -LogName ResticHub -MaxEvents 50 | Format-List TimeCreated, LevelDisplayName, Message
```

To read the last lines of the full log, use an elevated PowerShell:

```powershell
Get-ChildItem "$env:ProgramData\ResticHub\Agent-logs\agent-*.log" | Sort-Object Name | Select-Object -Last 1 | Get-Content -Tail 100
```

If the service cannot write a log file, it writes one warning to the event log. If it cannot write an event, it writes one warning to the log file. Then it tries again each 60 seconds. When the log file is available again, its first line tells how many events are missing. A log failure does not stop a job.

`install-service` registers the `ResticHub` log and the event source. If the registration fails, the command removes the registry keys that it created and does not change the service.

`uninstall-service` stops the service, removes it and then removes the event source. These items stay:

- The `ResticHub` event log and its events. Event Viewer shows the text of each event below the note "The description for Event ID ... cannot be found".
- The log files in `%ProgramData%\ResticHub\Agent-logs`.

To remove the event log, run `Remove-EventLog -LogName ResticHub` in an elevated Windows PowerShell 5.1. To remove the log files, delete the `Agent-logs` directory.

The log holds no repository password and no other secret.
