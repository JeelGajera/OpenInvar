// A using naming a namespace, an alias, and a static using — the three ways a
// C# file brings in a name it did not declare.
using Example.Models;
using Fmt = Example.Util.Formatter;
using static Example.Util.Helpers;

namespace Example
{
    // One half of a partial class. The other half declares Summarise, and the
    // call to it below has to resolve across both files.
    public partial class Report
    {
        private User subject;

        public string Render(Fmt formatter)
        {
            string label = formatter.Format(subject.Email);
            Log(label);
            return Summarise();
        }
    }
}
