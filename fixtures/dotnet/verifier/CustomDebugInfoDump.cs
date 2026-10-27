// Print the decoded custom debug information section of the dump.
//
// SRM gives the rows, the kind GUIDs and the blobs. SRM does not decode the blobs, so this
// file decodes them with BlobReader. The layouts come from the Portable PDB specification
// (dotnet/runtime 736475ca4a183e931d84cafb4954cff582b59ae9) and from Roslyn
// (e3c31f15b79e226656e3b082a921d4d60127b1fa): MetadataWriter.PortablePdb.cs and
// EditAndContinueMethodDebugInformation.cs. fixtures/README.md lists the lines.

using System;
using System.IO;
using System.IO.Compression;
using System.Reflection.Metadata;
using System.Reflection.Metadata.Ecma335;
using System.Security.Cryptography;
using System.Text;

namespace PortableVerifier;

/// <summary>Write the <c>cdi</c> lines of one Portable PDB.</summary>
internal static class CustomDebugInfoDump
{
    private static readonly Guid AsyncStepping = new("54FD2AC5-E925-401A-9C2A-F94F171072F8");
    private static readonly Guid HoistedScopes = new("6DA9A61E-F8C7-4874-BE62-68BC5630DF71");
    private static readonly Guid DynamicLocals = new("83C563C4-B4F3-47D5-B824-BA5441477EA8");
    private static readonly Guid TupleNames = new("ED9FDF71-8879-4747-8ED3-FE5EDE3CE710");
    private static readonly Guid DefaultNamespace = new("58B2EAB6-209F-4E4E-A22C-B2D0F910C782");
    private static readonly Guid EncLocalSlots = new("755F52A8-91C5-45BE-B4B8-209571E552BD");
    private static readonly Guid EncLambdas = new("A643004C-0240-496F-A783-30D64F4979DE");
    private static readonly Guid EncStates = new("8B78CD68-2EDE-420B-980B-E15884B8AAA3");
    private static readonly Guid SourceLink = new("CC110556-A091-4D38-9FEC-25AB9A351A6A");
    private static readonly Guid EmbeddedSource = new("0E8A571B-6926-466E-B4AD-8AB04611F5FE");
    private static readonly Guid MetadataReferences = new("7E4D4708-096E-4C5C-AEDA-CB10BA6A740D");
    private static readonly Guid CompilationOptions = new("B5FEEC05-8CD0-4A83-96DA-466284BB4BD8");
    private static readonly Guid TypeDocuments = new("932E74BC-DBA9-4478-8D46-0F32A7BAB3D3");
    private static readonly Guid PrimaryConstructor = new("9D40ACE1-C703-4D0E-BF41-7243060A8FB5");
    private static readonly Guid Sha256 = new("8829D00F-11B8-4213-878B-770E8597AC16");

    public static void Write(MetadataReader reader, StringBuilder output)
    {
        foreach (CustomDebugInformationHandle handle in reader.CustomDebugInformation)
        {
            CustomDebugInformation information = reader.GetCustomDebugInformation(handle);
            int id = MetadataTokens.GetRowNumber(handle);
            int parent = information.Parent.IsNil ? 0 : MetadataTokens.GetToken(information.Parent);
            Guid kind = information.Kind.IsNil ? Guid.Empty : reader.GetGuid(information.Kind);
            BlobReader blob = reader.GetBlobReader(information.Value);
            var lines = new StringBuilder();
            string codec = Decode(kind, blob, id, lines);
            output.Append($"cdi id={id} parent=0x{parent:x8} kind={kind:D} codec={codec}\n");
            output.Append(lines);
        }
    }

    /// <summary>Decode a blob, write its detail lines and return the codec name.</summary>
    private static string Decode(Guid kind, BlobReader blob, int id, StringBuilder lines)
    {
        void Line(string text) => lines.Append($"{text}\n");

        if (kind == AsyncStepping)
        {
            uint catchHandler = blob.ReadUInt32();
            Line($"cdi-async id={id} catch-handler={(catchHandler == 0 ? "none" : Hex(catchHandler - 1))}");
            while (blob.RemainingBytes > 0)
            {
                uint yieldOffset = blob.ReadUInt32();
                uint resumeOffset = blob.ReadUInt32();
                int method = blob.ReadCompressedInteger();
                Line($"cdi-async-step id={id} yield={Hex(yieldOffset)} resume={Hex(resumeOffset)} method={method}");
            }
            return "async-stepping";
        }
        if (kind == HoistedScopes)
        {
            while (blob.RemainingBytes > 0)
            {
                uint start = blob.ReadUInt32();
                uint length = blob.ReadUInt32();
                Line($"cdi-hoisted-scope id={id} start={Hex(start)} length={Hex(length)}");
            }
            return "hoisted-scopes";
        }
        if (kind == DynamicLocals)
        {
            var bits = new StringBuilder();
            while (blob.RemainingBytes > 0)
            {
                byte value = blob.ReadByte();
                for (int bit = 0; bit < 8; bit++)
                {
                    bits.Append((value & (1 << bit)) != 0 ? '1' : '0');
                }
            }
            Line($"cdi-dynamic id={id} flags={bits}");
            return "dynamic-locals";
        }
        if (kind == TupleNames)
        {
            while (blob.RemainingBytes > 0)
            {
                Line($"cdi-tuple-name id={id} name={Dump.Quote(ReadString(ref blob))}");
            }
            return "tuple-names";
        }
        if (kind == DefaultNamespace || kind == SourceLink)
        {
            Line($"cdi-text id={id} text={Dump.Quote(blob.ReadBytes(blob.RemainingBytes))}");
            return kind == SourceLink ? "source-link" : "default-namespace";
        }
        if (kind == EncLocalSlots)
        {
            DecodeSlots(blob, id, Line);
            return "enc-local-slots";
        }
        if (kind == EncLambdas)
        {
            DecodeLambdas(blob, id, Line);
            return "enc-lambdas";
        }
        if (kind == EncStates)
        {
            DecodeStates(blob, id, Line);
            return "enc-states";
        }
        if (kind == EmbeddedSource)
        {
            DecodeEmbeddedSource(blob, id, Line);
            return "embedded-source";
        }
        if (kind == MetadataReferences)
        {
            while (blob.RemainingBytes > 0)
            {
                byte[] file = ReadString(ref blob);
                byte[] aliases = ReadString(ref blob);
                byte flags = blob.ReadByte();
                uint timeStamp = blob.ReadUInt32();
                uint fileSize = blob.ReadUInt32();
                byte[] mvid = blob.ReadBytes(16);
                Line($"cdi-reference id={id} file={Dump.Quote(file)} aliases={Dump.Quote(aliases)} "
                    + $"flags=0x{flags:x2} time-stamp=0x{timeStamp:x8} file-size=0x{fileSize:x8} "
                    + $"mvid={Convert.ToHexString(mvid).ToLowerInvariant()}");
            }
            return "metadata-references";
        }
        if (kind == CompilationOptions)
        {
            while (blob.RemainingBytes > 0)
            {
                byte[] name = ReadString(ref blob);
                byte[] value = ReadString(ref blob);
                Line($"cdi-option id={id} name={Dump.Quote(name)} value={Dump.Quote(value)}");
            }
            return "compilation-options";
        }
        if (kind == TypeDocuments)
        {
            while (blob.RemainingBytes > 0)
            {
                Line($"cdi-type-document id={id} document={blob.ReadCompressedInteger()}");
            }
            return "type-documents";
        }
        if (kind == PrimaryConstructor)
        {
            if (blob.RemainingBytes != 0)
            {
                throw new InvalidDataException("primary constructor information is not empty");
            }
            return "primary-constructor";
        }
        byte[] raw = blob.ReadBytes(blob.RemainingBytes);
        Line($"cdi-raw id={id} size={Hex((uint)raw.Length)} bytes={Convert.ToHexString(raw).ToLowerInvariant()}");
        return "unknown";
    }

    /// <summary>Decode an Edit and Continue local slot map as Roslyn UncompressSlotMap does.</summary>
    private static void DecodeSlots(BlobReader blob, int id, Action<string> line)
    {
        // The 0xff byte and the negated baseline can only start the blob.
        int baseline = -1;
        if (blob.RemainingBytes > 0 && blob.ReadByte() == 0xff)
        {
            baseline = -blob.ReadCompressedInteger();
        }
        else
        {
            blob.Reset();
        }
        while (blob.RemainingBytes > 0)
        {
            byte value = blob.ReadByte();
            if (value == 0)
            {
                line($"cdi-enc-slot id={id} temporary");
                continue;
            }
            int kind = (value & 0x7f) - 1;
            int offset = blob.ReadCompressedInteger() + baseline;
            string ordinal = (value & 0x80) != 0 ? blob.ReadCompressedInteger().ToString() : "none";
            line($"cdi-enc-slot id={id} kind={kind} offset={offset} ordinal={ordinal}");
        }
    }

    /// <summary>Decode an Edit and Continue lambda map as Roslyn UncompressLambdaMap does.</summary>
    private static void DecodeLambdas(BlobReader blob, int id, Action<string> line)
    {
        int methodOrdinal = blob.ReadCompressedInteger() - 1;
        int baseline = -blob.ReadCompressedInteger();
        int closures = blob.ReadCompressedInteger();
        line($"cdi-enc-lambda-map id={id} method-ordinal={methodOrdinal}");
        for (int index = 0; index < closures; index++)
        {
            line($"cdi-enc-closure id={id} offset={blob.ReadCompressedInteger() + baseline}");
        }
        while (blob.RemainingBytes > 0)
        {
            int offset = blob.ReadCompressedInteger() + baseline;
            // Roslyn LambdaDebugInfo: -2 is "this only", -1 is static.
            int ordinal = blob.ReadCompressedInteger() - 2;
            string closure = ordinal switch
            {
                -2 => "this",
                -1 => "static",
                _ => ordinal.ToString(),
            };
            line($"cdi-enc-lambda id={id} offset={offset} closure={closure}");
        }
    }

    /// <summary>Decode an Edit and Continue state map as Roslyn UncompressStateMachineStates does.</summary>
    private static void DecodeStates(BlobReader blob, int id, Action<string> line)
    {
        int count = blob.ReadCompressedInteger();
        if (count == 0)
        {
            return;
        }
        int baseline = -blob.ReadCompressedInteger();
        for (int index = 0; index < count; index++)
        {
            int number = blob.ReadCompressedSignedInteger();
            int offset = baseline + blob.ReadCompressedInteger();
            line($"cdi-enc-state id={id} number={number} offset={offset}");
        }
    }

    /// <summary>Decode an embedded source and write its size and FNV-1a hash.</summary>
    private static void DecodeEmbeddedSource(BlobReader blob, int id, Action<string> line)
    {
        byte[] source = DecodeSource(blob, out bool raw);
        ulong hash = 0xcbf29ce484222325;
        foreach (byte value in source)
        {
            hash = unchecked((hash ^ value) * 0x100000001b3);
        }
        string name = raw ? "raw" : "deflate";
        line($"cdi-embedded-source id={id} format={name} size={Hex((uint)source.Length)} fnv1a=0x{hash:x16}");
    }

    /// <summary>Decode an embedded-source blob: raw bytes or a DEFLATE stream.</summary>
    private static byte[] DecodeSource(BlobReader blob, out bool raw)
    {
        int format = blob.ReadInt32();
        byte[] data = blob.ReadBytes(blob.RemainingBytes);
        raw = format == 0;
        if (raw)
        {
            return data;
        }
        using var output = new MemoryStream();
        using (var deflate = new DeflateStream(new MemoryStream(data), CompressionMode.Decompress))
        {
            deflate.CopyTo(output);
        }
        byte[] source = output.ToArray();
        if (source.Length != format)
        {
            throw new InvalidDataException("embedded source size differs from its format field");
        }
        return source;
    }

    /// <summary>
    /// Write a <c>source</c> line for each embedded source of each document: the check of the
    /// decoded source against the document hash. Only SHA-256 is computed.
    /// </summary>
    public static void WriteSources(MetadataReader reader, StringBuilder output)
    {
        foreach (DocumentHandle handle in reader.Documents)
        {
            Document document = reader.GetDocument(handle);
            Guid algorithm = document.HashAlgorithm.IsNil ? Guid.Empty : reader.GetGuid(document.HashAlgorithm);
            foreach (CustomDebugInformationHandle row in reader.GetCustomDebugInformation(handle))
            {
                CustomDebugInformation information = reader.GetCustomDebugInformation(row);
                if (information.Kind.IsNil || reader.GetGuid(information.Kind) != EmbeddedSource)
                {
                    continue;
                }
                byte[] source = DecodeSource(reader.GetBlobReader(information.Value), out _);
                string check = algorithm != Sha256
                    ? "not-verified"
                    : SHA256.HashData(source).AsSpan().SequenceEqual(reader.GetBlobBytes(document.Hash))
                        ? "verified"
                        : "mismatch";
                output.Append($"source document={MetadataTokens.GetRowNumber(handle)} "
                    + $"cdi={MetadataTokens.GetRowNumber(row)} size={Hex((uint)source.Length)} check={check}\n");
            }
        }
    }

    /// <summary>Read a NUL-terminated string without its terminator.</summary>
    private static byte[] ReadString(ref BlobReader blob)
    {
        int end = blob.IndexOf(0);
        if (end < 0)
        {
            throw new InvalidDataException("a string of a custom debug information blob has no NUL terminator");
        }
        byte[] text = blob.ReadBytes(end);
        blob.ReadByte();
        return text;
    }

    private static string Hex(uint value) => $"0x{value:x}";
}
