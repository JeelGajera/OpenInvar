<?php
// UserService is declared in another file. Tier 2 does not resolve across
// files, so this reference records no cross-file edge — and `status` reports
// the tier so nobody mistakes that silence for "nothing uses UserService".

namespace App;

use App\Service\UserService;

class Elsewhere
{
    public function run(): string
    {
        $service = new UserService();
        return $service->handle();
    }
}
