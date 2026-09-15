// UserService is declared in another file. Swift has no import statement for a
// file in the same target — the name is simply in scope — so there is nothing
// here for a resolver to follow even in principle, and Tier 2 records no
// cross-file edge. `status` reports the tier so that silence is not mistaken
// for "nothing uses UserService".

struct Elsewhere {
    func run() -> String {
        let service = UserService()
        return service.handle()
    }
}
