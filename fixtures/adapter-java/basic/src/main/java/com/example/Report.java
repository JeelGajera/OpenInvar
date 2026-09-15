package com.example;

// Three resolution paths in one file:
//   - a single-type import, across packages
//   - an on-demand import, which only resolves because exactly one package
//     offers the name
//   - a static import, where a bare call names a member of another type
import com.example.model.User;
import com.example.util.*;
import static com.example.util.Helpers.log;

public class Report extends Elsewhere {
    private User subject;

    public String render(Formatter formatter) {
        String label = formatter.format(subject.email);
        log(label);
        // Inherited from Elsewhere, which is in another file: found by walking
        // supertypes rather than by matching the name anywhere it appears.
        run();
        return subject.name;
    }

    // A plain return type, unwrapped by a generic or an array. This recorded
    // no edge to User at all, while a `List<User>` in the same position
    // recorded one: the walk that collected type names skipped the node it
    // started from, and for an unwrapped type that node is the name.
    public User getSubject() {
        return subject;
    }
}
