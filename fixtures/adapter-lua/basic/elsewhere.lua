-- user_service is loaded at runtime: `require` returns whatever table that
-- file's last statement produced, so what `service` holds is not decidable
-- from this file. Tier 2 records no cross-file edge, and `status` reports the
-- tier so that silence is not mistaken for "nothing uses UserService".
local service = require("user_service")

local function run()
  return service.handle()
end

return run
