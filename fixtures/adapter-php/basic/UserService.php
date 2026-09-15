<?php
// Everything below is resolvable inside this one file, which is exactly the
// limit of Tier 2: the analyzer records what it can see here and nothing more.

namespace App\Service;

trait Auditing
{
    public function describe(): string
    {
        return "user service";
    }
}

class AuditLog
{
    public function record(string $message): string
    {
        return $message;
    }
}

class UserService
{
    use Auditing;

    private AuditLog $log;

    public function __construct()
    {
        $this->log = new AuditLog();
    }

    public function handle(): string
    {
        // A call to a method defined in this file.
        return $this->describe();
    }
}
