using System;
using System.ComponentModel;
using System.IO;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading.Tasks;

namespace Daruda.Scripts {
    public sealed class CommandResult {
        public int ExitCode { get; set; }
        public string Stdout { get; set; }
        public string Stderr { get; set; }
    }

    public static class HiddenConsole {
        private const uint CreateNewConsole = 0x00000010;
        private const uint ExtendedStartupInfoPresent = 0x00080000;
        private const uint StartfUseShowWindow = 0x00000001;
        private const uint StartfUseStdHandles = 0x00000100;
        private const int HandleListAttribute = 0x00020002;

        [StructLayout(LayoutKind.Sequential)]
        private struct StartupInfo {
            public int Size;
            public IntPtr Reserved, Desktop, Title;
            public uint X, Y, XSize, YSize, XCountChars, YCountChars, FillAttribute, Flags;
            public ushort ShowWindow, ReservedSize;
            public IntPtr ReservedBytes, Input, Output, Error;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct StartupInfoEx {
            public StartupInfo Startup;
            public IntPtr Attributes;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ProcessInfo {
            public IntPtr Process, Thread;
            public uint ProcessId, ThreadId;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool InitializeProcThreadAttributeList(
            IntPtr list, int count, int flags, ref IntPtr size);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool UpdateProcThreadAttribute(IntPtr list, uint flags,
            IntPtr attribute, IntPtr value, IntPtr size, IntPtr previous, IntPtr returnedSize);
        [DllImport("kernel32.dll")]
        private static extern void DeleteProcThreadAttributeList(IntPtr list);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CreateProcessW(string application, StringBuilder command,
            IntPtr processSecurity, IntPtr threadSecurity, bool inheritHandles, uint flags,
            IntPtr environment, string directory, ref StartupInfoEx startup, out ProcessInfo process);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);
        [DllImport("kernel32.dll")]
        private static extern bool CloseHandle(IntPtr handle);

        // Windows argv quoting, with no shell expansion of argument text.
        private static string Quote(string value) {
            var quoted = new StringBuilder("\"");
            int slashes = 0;
            foreach (char character in value) {
                if (character == '\\') { slashes++; continue; }
                quoted.Append('\\', character == '"' ? slashes * 2 + 1 : slashes);
                quoted.Append(character);
                slashes = 0;
            }
            quoted.Append('\\', slashes * 2);
            return quoted.Append('"').ToString();
        }

        private static ProcessInfo Start(string executable, string[] arguments, string directory,
                                         IntPtr input, IntPtr output, IntPtr error) {
            var command = new StringBuilder(Quote(executable));
            foreach (string argument in arguments) command.Append(' ').Append(Quote(argument));
            var size = IntPtr.Zero;
            InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref size);
            if (size == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            IntPtr list = Marshal.AllocHGlobal(size);
            IntPtr handles = IntPtr.Zero;
            bool initialized = false;
            try {
                if (!InitializeProcThreadAttributeList(list, 1, 0, ref size))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                initialized = true;
                handles = Marshal.AllocHGlobal(IntPtr.Size * 3);
                Marshal.WriteIntPtr(handles, 0, input);
                Marshal.WriteIntPtr(handles, IntPtr.Size, output);
                Marshal.WriteIntPtr(handles, IntPtr.Size * 2, error);
                // Inherit only streams, so unrelated pipe handles cannot keep readers alive.
                if (!UpdateProcThreadAttribute(list, 0, new IntPtr(HandleListAttribute), handles,
                        new IntPtr(IntPtr.Size * 3), IntPtr.Zero, IntPtr.Zero))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                var startup = new StartupInfoEx {
                    Startup = new StartupInfo {
                        Size = Marshal.SizeOf(typeof(StartupInfoEx)),
                        Flags = StartfUseShowWindow | StartfUseStdHandles,
                        ShowWindow = 0, // SW_HIDE applies when the console is created.
                        Input = input, Output = output, Error = error
                    },
                    Attributes = list
                };
                ProcessInfo process;
                // A real hidden console lets raw child tools inherit it even from a GUI host.
                if (!CreateProcessW(executable, command, IntPtr.Zero, IntPtr.Zero, true,
                        CreateNewConsole | ExtendedStartupInfoPresent, IntPtr.Zero, directory,
                        ref startup, out process))
                    throw new Win32Exception(Marshal.GetLastWin32Error());
                CloseHandle(process.Thread);
                return process;
            } finally {
                if (initialized) DeleteProcThreadAttributeList(list);
                if (handles != IntPtr.Zero) Marshal.FreeHGlobal(handles);
                Marshal.FreeHGlobal(list);
            }
        }

        private static string Read(Stream stream, TextWriter destination, bool capture) {
            var captured = new StringBuilder();
            using (var reader = new StreamReader(stream, Encoding.UTF8)) {
                string line;
                while ((line = reader.ReadLine()) != null) {
                    if (capture) captured.AppendLine(line);
                    else destination.WriteLine(line);
                }
            }
            return captured.ToString();
        }

        public static CommandResult Run(string executable, string[] arguments, string directory,
                                        bool captureOutput) {
            using (var input = new AnonymousPipeServerStream(PipeDirection.Out, HandleInheritability.Inheritable))
            using (var output = new AnonymousPipeServerStream(PipeDirection.In, HandleInheritability.Inheritable))
            using (var error = new AnonymousPipeServerStream(PipeDirection.In, HandleInheritability.Inheritable)) {
                var process = Start(executable, arguments, directory,
                    input.ClientSafePipeHandle.DangerousGetHandle(),
                    output.ClientSafePipeHandle.DangerousGetHandle(),
                    error.ClientSafePipeHandle.DangerousGetHandle());
                try {
                    input.DisposeLocalCopyOfClientHandle();
                    output.DisposeLocalCopyOfClientHandle();
                    error.DisposeLocalCopyOfClientHandle();
                    input.Dispose(); // Validation commands receive EOF rather than interactive input.
                    var stdout = Task.Run(() => Read(output, Console.Out, captureOutput));
                    var stderr = Task.Run(() => Read(error, Console.Error, captureOutput));
                    if (WaitForSingleObject(process.Process, uint.MaxValue) != 0)
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                    uint exitCode;
                    if (!GetExitCodeProcess(process.Process, out exitCode))
                        throw new Win32Exception(Marshal.GetLastWin32Error());
                    return new CommandResult {
                        ExitCode = unchecked((int)exitCode),
                        Stdout = stdout.Result,
                        Stderr = stderr.Result
                    };
                } finally {
                    CloseHandle(process.Process);
                }
            }
        }
    }
}
