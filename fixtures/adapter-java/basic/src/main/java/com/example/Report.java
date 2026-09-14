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
}
