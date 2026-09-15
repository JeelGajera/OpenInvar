-- Everything below is resolvable inside this one file, which is exactly the
-- limit of Tier 2: the analyzer records what it can see here and nothing more.

local AuditLog = {}

function AuditLog.record(message)
  return message
end

local UserService = {}

function UserService.describe()
  return "user service"
end

function UserService.handle()
  -- A call to a function defined in this file.
  return UserService.describe()
end

return UserService
