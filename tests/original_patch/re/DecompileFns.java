// Decompile a list of functions without whole-program analysis.
// args: <out dir> <addr> [<addr> ...]
import java.io.File;
import java.io.PrintWriter;
import ghidra.app.cmd.disassemble.DisassembleCommand;
import ghidra.app.cmd.function.CreateFunctionCmd;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;

public class DecompileFns extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        File out = new File(args[0]);
        out.mkdirs();
        DecompInterface ifc = new DecompInterface();
        ifc.openProgram(currentProgram);
        for (int i = 1; i < args.length; i++) {
            Address a = toAddr(args[i]);
            new DisassembleCommand(a, null, true).applyTo(currentProgram, monitor);
            Function f = getFunctionAt(a);
            if (f == null) {
                new CreateFunctionCmd(a).applyTo(currentProgram, monitor);
                f = getFunctionAt(a);
            }
            if (f == null) { println("NOFUNC " + args[i]); continue; }
            DecompileResults r = ifc.decompileFunction(f, 600, monitor);
            File o = new File(out, args[i] + ".c");
            try (PrintWriter w = new PrintWriter(o)) {
                if (r.decompileCompleted()) w.print(r.getDecompiledFunction().getC());
                else w.print("// FAILED: " + r.getErrorMessage());
            }
            println("DONE " + args[i] + " " + f.getName() + " ok=" + r.decompileCompleted());
        }
        ifc.dispose();
    }
}
