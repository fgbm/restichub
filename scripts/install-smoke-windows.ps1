# Stops the agent service and waits for the stopped state. A stop sent right after a start can
# fail while the service manager still changes the state, so the stop is retried.
function Stop-AgentService {
    $deadline = (Get-Date).AddSeconds(90)
    while ((Get-Service restichub-agent).Status -ne 'Stopped') {
        if ((Get-Date) -gt $deadline) {
            throw "the service did not stop, state: $((Get-Service restichub-agent).Status)"
        }
        try {
            Stop-Service restichub-agent -ErrorAction Stop
        } catch {
            Write-Host "stop retry, state $((Get-Service restichub-agent).Status): $($_.Exception.Message) $($_.Exception.InnerException.Message)"
            Start-Sleep -Seconds 1
        }
    }
}
