// Print the canonical dump of a Portable PDB through System.Reflection.Metadata (SRM).
//
// fixtures/README.md ("Portable PDB verifier") describes the format. The `verify-portable`
// command of pdbrs-fixtures prints the same format with pdbrs and compares the two outputs.
//
// SRM does not expose the stream headers, the #~ header fields or the external row counts of
// the #Pdb stream. This program reads these values from the bytes. Then it checks them against
// SRM: the heap offsets, the #Pdb offset, the local row counts and the row size of each table.
// The row values come from SRM handles. SRM does not expose the VariableList and ConstantList
// columns or the StateMachineMethod rows, so the program reads them from the row bytes and
// checks them against the SRM views of the same data.

using System;
using System.Collections.Generic;
using System.Collections.Immutable;
using System.IO;
using System.Reflection;
using System.Reflection.Metadata;
using System.Reflection.Metadata.Ecma335;
using System.Runtime.InteropServices;
using System.Text;

namespace PortableVerifier;

internal static class Program
{
    private static int Main(string[] args)
    {
        if (args is ["--version"])
        {
            // Print the versions that the tools manifest records.
            var assembly = typeof(MetadataReader).Assembly;
            string? version = assembly
                .GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion;
            Console.WriteLine($"System.Reflection.Metadata {version ?? assembly.GetName().Version?.ToString()}; "
                + RuntimeInformation.FrameworkDescription);
            return 0;
        }
        if (args.Length != 1)
        {
            Console.Error.WriteLine("usage: PortableVerifier <file.pdb> | --version");
            return 2;
        }
        try
        {
            byte[] bytes = File.ReadAllBytes(args[0]);
            using MetadataReaderProvider provider =
                MetadataReaderProvider.FromPortablePdbImage(ImmutableArray.Create(bytes));
            MetadataReader reader = provider.GetMetadataReader();
            var output = new StringBuilder();
            new Dump(bytes, reader, output).Write();
            byte[] text = Encoding.UTF8.GetBytes(output.ToString());
            using Stream stdout = Console.OpenStandardOutput();
            stdout.Write(text, 0, text.Length);
            return 0;
        }
        catch (Exception error) when (error is BadImageFormatException or InvalidDataException
            or IOException or ArgumentException or InvalidOperationException)
        {
            Console.Error.WriteLine($"error: {error.Message}");
            return 1;
        }
    }
}

/// <summary>Write the dump lines of one Portable PDB.</summary>
internal sealed class Dump
{
    private const int MethodDef = 0x06;
    private const int Document = 0x30;
    private const int MethodDebugInformation = 0x31;
    private const int LocalScope = 0x32;
    private const int LocalVariable = 0x33;
    private const int LocalConstant = 0x34;
    private const int ImportScope = 0x35;
    private const int StateMachineMethod = 0x36;
    private const int CustomDebugInformation = 0x37;

    /// <summary>The eight debug tables in table number order.</summary>
    private static readonly (int Table, string Name)[] DebugTables =
    {
        (Document, "Document"),
        (MethodDebugInformation, "MethodDebugInformation"),
        (LocalScope, "LocalScope"),
        (LocalVariable, "LocalVariable"),
        (LocalConstant, "LocalConstant"),
        (ImportScope, "ImportScope"),
        (StateMachineMethod, "StateMachineMethod"),
        (CustomDebugInformation, "CustomDebugInformation"),
    };

    /// <summary>The tables of the HasCustomDebugInformation coded index in tag order.</summary>
    private static readonly int[] HasCustomDebugInformationTables =
    {
        0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B,
        0x20, 0x23, 0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B, 0x30, 0x32, 0x33, 0x34, 0x35,
    };

    private readonly byte[] bytes;
    private readonly MetadataReader reader;
    private readonly StringBuilder output;
    private readonly List<(string Name, int Offset, int Size)> streams = new();
    private readonly long[] rowCounts = new long[64];

    public Dump(byte[] bytes, MetadataReader reader, StringBuilder output)
    {
        this.bytes = bytes;
        this.reader = reader;
        this.output = output;
    }

    public void Write()
    {
        Line("portable-dump version=1");
        ReadStreamHeaders();
        Line($"metadata version={Quote(Encoding.UTF8.GetBytes(reader.MetadataVersion))} streams={streams.Count}");
        foreach ((string name, int offset, int size) in streams)
        {
            Line($"stream name={name} offset={Hex(offset)} size={Hex(size)}");
        }
        WriteHeaps();
        WritePdb();
        WriteTables();
        WriteDocuments();
        WriteMethodDebugInformation();
        WriteLocalScopes();
        WriteLocalVariables();
        WriteLocalConstants();
        WriteImportScopes();
        WriteStateMachineMethods();
        WriteCustomDebugInformation();
    }

    private void ReadStreamHeaders()
    {
        int position = 16 + (int)U32(12) + 2;
        int count = U16(position);
        position += 2;
        for (int index = 0; index < count; index++)
        {
            int offset = (int)U32(position);
            int size = (int)U32(position + 4);
            position += 8;
            int end = Array.IndexOf(bytes, (byte)0, position);
            streams.Add((Encoding.ASCII.GetString(bytes, position, end - position), offset, size));
            position = (end + 4) & ~3;
        }
    }

    private (int Offset, int Size)? FindStream(string name)
    {
        foreach ((string streamName, int offset, int size) in streams)
        {
            if (streamName == name)
            {
                return (offset, size);
            }
        }
        return null;
    }

    private (int Offset, int Size) Stream(string name) =>
        FindStream(name) ?? throw new InvalidDataException($"the {name} stream is absent");

    /// <summary>
    /// Write the size of each heap. The size is the size of its stream, or 0 if the stream is
    /// absent. SRM removes the zero padding after the last string of #Strings, so for that heap
    /// the check accepts a smaller SRM size if only zero bytes follow it.
    /// </summary>
    private void WriteHeaps()
    {
        foreach ((HeapIndex heap, string name) in new[]
        {
            (HeapIndex.String, "#Strings"),
            (HeapIndex.UserString, "#US"),
            (HeapIndex.Blob, "#Blob"),
            (HeapIndex.Guid, "#GUID"),
        })
        {
            (int offset, int size) = FindStream(name) ?? (0, 0);
            int srmSize = reader.GetHeapSize(heap);
            if (srmSize > 0)
            {
                Check(reader.GetHeapMetadataOffset(heap) == offset, $"{name} offset");
            }
            bool padded = heap == HeapIndex.String && srmSize <= size
                && Array.TrueForAll(bytes[(offset + srmSize)..(offset + size)], value => value == 0);
            Check(srmSize == size || padded, $"{name} size");
            Line($"heap name={name} size={Hex(size)}");
        }
    }

    private void WritePdb()
    {
        DebugMetadataHeader header = reader.DebugMetadataHeader
            ?? throw new InvalidDataException("SRM reports no #Pdb stream");
        (int offset, _) = Stream("#Pdb");
        Check(header.IdStartOffset == offset, "#Pdb offset");
        int entryPoint = header.EntryPoint.IsNil ? 0 : MetadataTokens.GetToken(header.EntryPoint);
        Check(U32(offset + 20) == (uint)entryPoint, "#Pdb entry point");
        var id = new StringBuilder();
        foreach (byte value in header.Id)
        {
            id.Append(value.ToString("x2"));
        }
        Line($"pdb id={id} entry-point={Token(entryPoint)}");
        ulong mask = U64(offset + 24);
        int position = offset + 32;
        for (int table = 0; table < 64; table++)
        {
            if ((mask & (1UL << table)) != 0)
            {
                rowCounts[table] = U32(position);
                position += 4;
                Line($"pdb-rows table={Hex(table, 2)} rows={rowCounts[table]}");
            }
        }
    }

    private void WriteTables()
    {
        (int offset, _) = Stream("#~");
        byte heapSizes = bytes[offset + 6];
        ulong valid = U64(offset + 8);
        ulong sorted = U64(offset + 16);
        Line($"tables version={bytes[offset + 4]}.{bytes[offset + 5]} heap-sizes={Hex(heapSizes, 2)} "
            + $"valid={Mask(valid)} sorted={Mask(sorted)}");
        int position = offset + 24;
        for (int table = 0; table < 64; table++)
        {
            if ((valid & (1UL << table)) != 0)
            {
                rowCounts[table] = U32(position);
                position += 4;
            }
        }

        int stringWidth = (heapSizes & 1) != 0 ? 4 : 2;
        int guidWidth = (heapSizes & 2) != 0 ? 4 : 2;
        int blobWidth = (heapSizes & 4) != 0 ? 4 : 2;
        Line($"index heap=#Strings width={stringWidth}");
        Line($"index heap=#GUID width={guidWidth}");
        Line($"index heap=#Blob width={blobWidth}");
        foreach ((int table, string name) in new[]
        {
            (MethodDef, "MethodDef"),
            (Document, "Document"),
            (LocalVariable, "LocalVariable"),
            (LocalConstant, "LocalConstant"),
            (ImportScope, "ImportScope"),
        })
        {
            Line($"index table={name} width={TableWidth(table)}");
        }
        long codedRows = 0;
        foreach (int table in HasCustomDebugInformationTables)
        {
            codedRows = Math.Max(codedRows, rowCounts[table]);
        }
        int codedWidth = codedRows < (1 << 11) ? 2 : 4;
        Line($"index coded=HasCustomDebugInformation width={codedWidth}");

        int method = TableWidth(MethodDef);
        var rowSizes = new Dictionary<int, int>
        {
            [Document] = blobWidth + guidWidth + blobWidth + guidWidth,
            [MethodDebugInformation] = TableWidth(Document) + blobWidth,
            [LocalScope] = method + TableWidth(ImportScope) + TableWidth(LocalVariable)
                + TableWidth(LocalConstant) + 8,
            [LocalVariable] = 4 + stringWidth,
            [LocalConstant] = stringWidth + blobWidth,
            [ImportScope] = TableWidth(ImportScope) + blobWidth,
            [StateMachineMethod] = 2 * method,
            [CustomDebugInformation] = codedWidth + guidWidth + blobWidth,
        };
        foreach ((int table, string name) in DebugTables)
        {
            var index = (TableIndex)table;
            int rows = reader.GetTableRowCount(index);
            int rowSize = reader.GetTableRowSize(index);
            Check(rows == rowCounts[table], $"{name} row count");
            Check(rowSize == rowSizes[table], $"{name} row size");
            string location = rows > 0 ? $" offset={Hex(reader.GetTableMetadataOffset(index))}" : "";
            Line($"table name={name} id={Hex(table, 2)} rows={rows} row-size={Hex(rowSize)}{location}");
        }
    }

    private void WriteDocuments()
    {
        foreach (DocumentHandle handle in reader.Documents)
        {
            System.Reflection.Metadata.Document document = reader.GetDocument(handle);
            Line($"row table=Document id={Row(handle)} name={Heap((BlobHandle)document.Name)} "
                + $"hash-algorithm={Heap(document.HashAlgorithm)} hash={Heap(document.Hash)} "
                + $"language={Heap(document.Language)}");
        }
    }

    private void WriteMethodDebugInformation()
    {
        foreach (MethodDebugInformationHandle handle in reader.MethodDebugInformation)
        {
            System.Reflection.Metadata.MethodDebugInformation information =
                reader.GetMethodDebugInformation(handle);
            Line($"row table=MethodDebugInformation id={Row(handle)} "
                + $"document={Row(information.Document)} "
                + $"sequence-points={Heap(information.SequencePointsBlob)}");
        }
    }

    private void WriteLocalScopes()
    {
        int method = TableWidth(MethodDef);
        int import = TableWidth(ImportScope);
        int variableWidth = TableWidth(LocalVariable);
        foreach (LocalScopeHandle handle in reader.LocalScopes)
        {
            System.Reflection.Metadata.LocalScope scope = reader.GetLocalScope(handle);
            int row = Row(handle);
            int position = RowOffset(LocalScope, row) + method + import;
            uint variables = Read(position, variableWidth);
            uint constants = Read(position + variableWidth, TableWidth(LocalConstant));
            CheckFirst(scope.GetLocalVariables(), variables, "LocalScope VariableList");
            CheckFirst(scope.GetLocalConstants(), constants, "LocalScope ConstantList");
            Line($"row table=LocalScope id={row} method={Row(scope.Method)} "
                + $"import-scope={Row(scope.ImportScope)} variable-list={variables} "
                + $"constant-list={constants} start-offset={Hex(scope.StartOffset)} "
                + $"length={Hex(scope.Length)}");
        }
    }

    private void WriteLocalVariables()
    {
        foreach (LocalVariableHandle handle in reader.LocalVariables)
        {
            System.Reflection.Metadata.LocalVariable variable = reader.GetLocalVariable(handle);
            Line($"row table=LocalVariable id={Row(handle)} "
                + $"attributes={Hex((int)variable.Attributes, 4)} index={variable.Index} "
                + $"name={Heap(variable.Name)}");
        }
    }

    private void WriteLocalConstants()
    {
        foreach (LocalConstantHandle handle in reader.LocalConstants)
        {
            System.Reflection.Metadata.LocalConstant constant = reader.GetLocalConstant(handle);
            Line($"row table=LocalConstant id={Row(handle)} name={Heap(constant.Name)} "
                + $"signature={Heap(constant.Signature)}");
        }
    }

    private void WriteImportScopes()
    {
        foreach (ImportScopeHandle handle in reader.ImportScopes)
        {
            System.Reflection.Metadata.ImportScope scope = reader.GetImportScope(handle);
            Line($"row table=ImportScope id={Row(handle)} parent={Row(scope.Parent)} "
                + $"imports={Heap(scope.ImportsBlob)}");
        }
    }

    private void WriteStateMachineMethods()
    {
        int method = TableWidth(MethodDef);
        var index = (TableIndex)StateMachineMethod;
        for (int row = 1; row <= reader.GetTableRowCount(index); row++)
        {
            int position = RowOffset(StateMachineMethod, row);
            uint moveNext = Read(position, method);
            uint kickoff = Read(position + method, method);
            MethodDefinitionHandle found = reader
                .GetMethodDebugInformation(MetadataTokens.MethodDefinitionHandle((int)moveNext))
                .GetStateMachineKickoffMethod();
            Check(Row(found) == kickoff, $"StateMachineMethod row {row} kickoff");
            Line($"row table=StateMachineMethod id={row} move-next={moveNext} kickoff={kickoff}");
        }
    }

    private void WriteCustomDebugInformation()
    {
        foreach (CustomDebugInformationHandle handle in reader.CustomDebugInformation)
        {
            System.Reflection.Metadata.CustomDebugInformation information =
                reader.GetCustomDebugInformation(handle);
            int parent = information.Parent.IsNil ? 0 : MetadataTokens.GetToken(information.Parent);
            Line($"row table=CustomDebugInformation id={Row(handle)} parent={Token(parent)} "
                + $"kind={Heap(information.Kind)} value={Heap(information.Value)}");
        }
    }

    private int TableWidth(int table) => rowCounts[table] < 0x10000 ? 2 : 4;

    private int RowOffset(int table, int row) =>
        reader.GetTableMetadataOffset((TableIndex)table)
        + (row - 1) * reader.GetTableRowSize((TableIndex)table);

    private uint Read(int position, int width) => width == 2 ? U16(position) : U32(position);

    private static void CheckFirst(LocalVariableHandleCollection handles, uint raw, string what)
    {
        foreach (LocalVariableHandle handle in handles)
        {
            Check(Row(handle) == raw, what);
            return;
        }
    }

    private static void CheckFirst(LocalConstantHandleCollection handles, uint raw, string what)
    {
        foreach (LocalConstantHandle handle in handles)
        {
            Check(Row(handle) == raw, what);
            return;
        }
    }

    private static void Check(bool condition, string what)
    {
        if (!condition)
        {
            throw new InvalidDataException($"SRM and the raw bytes differ: {what}");
        }
    }

    private void Line(string text) => output.Append(text).Append('\n');

    private ushort U16(int position) => BitConverter.ToUInt16(bytes, position);

    private uint U32(int position) => BitConverter.ToUInt32(bytes, position);

    private ulong U64(int position) => BitConverter.ToUInt64(bytes, position);

    private static int Row(EntityHandle handle) => handle.IsNil ? 0 : MetadataTokens.GetRowNumber(handle);

    /// <summary>A #Blob offset in hexadecimal.</summary>
    private static string Heap(BlobHandle handle) => Hex(MetadataTokens.GetHeapOffset(handle));

    /// <summary>A 1-based #GUID index in decimal, 0 for nil.</summary>
    private static int Heap(GuidHandle handle) => MetadataTokens.GetHeapOffset(handle);

    /// <summary>A #Strings offset in hexadecimal.</summary>
    private static string Heap(StringHandle handle) => Hex(MetadataTokens.GetHeapOffset(handle));

    private static string Hex(long value) => $"0x{value:x}";

    private static string Hex(long value, int digits) => "0x" + value.ToString($"x{digits}");

    private static string Token(int token) => "0x" + token.ToString("x8");

    private static string Mask(ulong value) => "0x" + value.ToString("x16");

    private static string Quote(byte[] text)
    {
        var quoted = new StringBuilder("\"");
        foreach (byte value in text)
        {
            if (value is >= 0x20 and < 0x7f && value != (byte)'"' && value != (byte)'\\')
            {
                quoted.Append((char)value);
            }
            else
            {
                quoted.Append($"\\x{value:x2}");
            }
        }
        return quoted.Append('"').ToString();
    }
}
