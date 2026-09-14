package com.example;

// Everything here is resolved: the analyzer follows imports, binds each
// receiver to its declared type, and walks supertypes to find an inherited
// method. What it cannot resolve — String, the standard library — records no
// edge rather than a guessed one.
public interface Auditable {
    String describe();
}

class AuditLog {
    void record(String message) {
    }
}

public class UserService implements Auditable {
    private AuditLog log;

    public String describe() {
        return "user service";
    }

    public void handle() {
        // A call to a method defined in this file.
        describe();
        // A call on a field, resolved through the field's declared type.
        log.record("handled");
    }
}
