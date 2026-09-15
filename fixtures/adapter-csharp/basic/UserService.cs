// Everything here is resolved: the analyzer follows usings, binds each
// receiver to its declared type, and merges the halves of a partial class.
namespace Example
{
    public interface IAuditable
    {
        string Describe();
    }

    class AuditLog
    {
        public void Record(string message) { }
    }

    public class UserService : IAuditable
    {
        private AuditLog log;

        public string Describe()
        {
            return "user service";
        }

        public void Handle()
        {
            // A call to a method defined in this file.
            Describe();
            // A call on a field, resolved through the field's declared type.
            log.Record("handled");
        }
    }
}
