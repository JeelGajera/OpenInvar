// UserService is declared in another file of the same namespace, so C# needs
// no using for it — and neither does the resolver. `var` here names its type
// in the initialiser, which is read rather than inferred.
namespace Example
{
    public class Elsewhere
    {
        public void Run()
        {
            var service = new UserService();
            service.Handle();
        }
    }
}
