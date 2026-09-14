package com.example;

// UserService is declared in another file of the same package, so Java needs
// no import for it — and neither does the resolver. Both edges below are
// cross-file, which is exactly what Tier 2 could never record.
public class Elsewhere {
    public void run() {
        UserService service = new UserService();
        service.handle();
    }
}
