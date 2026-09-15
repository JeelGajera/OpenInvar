// Everything below is resolvable inside this one file, which is exactly the
// limit of Tier 2: the analyzer records what it can see here and nothing more.

protocol Auditing {
    func describe() -> String
}

struct AuditLog {
    func record(_ message: String) -> String {
        return message
    }
}

final class UserService: Auditing {
    private let log = AuditLog()

    func describe() -> String {
        return "user service"
    }

    func handle() -> String {
        // A call to a method defined in this file.
        return describe()
    }
}
