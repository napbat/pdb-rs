// Print the decoded debug table sections of the dump (#23): documents, sequence points, local
// scopes, import scopes and state machine methods.
//
// SRM decodes the document names, the sequence points and the imports. SRM does not decode
// local constant signatures, so this file decodes them with BlobReader. The layout comes from
// the Portable PDB specification (dotnet/runtime 736475ca4a183e931d84cafb4954cff582b59ae9).
// fixtures/README.md lists the lines.

using System;
using System.IO;
using System.Reflection.Metadata;
using System.Reflection.Metadata.Ecma335;
using System.Text;

namespace PortableVerifier;

/// <summary>Write the decoded debug table lines of one Portable PDB.</summary>
internal static class DebugDump
{
    public static void Write(MetadataReader reader, StringBuilder output)
    {
        void Line(string text) => output.Append(text).Append('\n');

        WriteDocuments(reader, Line);
        WriteSequencePoints(reader, Line);
        WriteScopes(reader, Line);
        WriteImportScopes(reader, Line);
        WriteStateMachines(reader, Line);
    }

    private static void WriteDocuments(MetadataReader reader, Action<string> line)
    {
        foreach (DocumentHandle handle in reader.Documents)
        {
            Document document = reader.GetDocument(handle);
            string name = Dump.Quote(Encoding.UTF8.GetBytes(reader.GetString(document.Name)));
            line($"document id={Row(handle)} name={name} language={Guid(reader, document.Language)} "
                + $"hash-algorithm={Guid(reader, document.HashAlgorithm)} "
                + $"hash={HexBytes(reader.GetBlobBytes(document.Hash))}");
        }
    }

    private static void WriteSequencePoints(MetadataReader reader, Action<string> line)
    {
        foreach (MethodDebugInformationHandle handle in reader.MethodDebugInformation)
        {
            MethodDebugInformation information = reader.GetMethodDebugInformation(handle);
            if (information.SequencePointsBlob.IsNil)
            {
                continue;
            }
            int method = Row(handle);
            int count = 0;
            foreach (SequencePoint _ in information.GetSequencePoints())
            {
                count++;
            }
            line($"method-debug id={method} document={Row(information.Document)} "
                + $"local-signature={Row(information.LocalSignature)} points={count}");
            foreach (SequencePoint point in information.GetSequencePoints())
            {
                string prefix = $"sequence-point method={method} il=0x{point.Offset:x} "
                    + $"document={Row(point.Document)}";
                if (point.IsHidden)
                {
                    line($"{prefix} hidden");
                }
                else
                {
                    line($"{prefix} start-line={point.StartLine} start-column={point.StartColumn} "
                        + $"end-line={point.EndLine} end-column={point.EndColumn}");
                }
            }
        }
    }

    private static void WriteScopes(MetadataReader reader, Action<string> line)
    {
        foreach (LocalScopeHandle handle in reader.LocalScopes)
        {
            LocalScope scope = reader.GetLocalScope(handle);
            int id = Row(handle);
            LocalVariableHandleCollection variables = scope.GetLocalVariables();
            LocalConstantHandleCollection constants = scope.GetLocalConstants();
            line($"scope id={id} method={Row(scope.Method)} import-scope={Row(scope.ImportScope)} "
                + $"start=0x{scope.StartOffset:x} end=0x{scope.EndOffset:x} "
                + $"variables={variables.Count} constants={constants.Count}");
            foreach (LocalVariableHandle variableHandle in variables)
            {
                LocalVariable variable = reader.GetLocalVariable(variableHandle);
                line($"scope-variable scope={id} id={Row(variableHandle)} index={variable.Index} "
                    + $"attributes=0x{(int)variable.Attributes:x4} name={Text(reader, variable.Name)}");
            }
            foreach (LocalConstantHandle constantHandle in constants)
            {
                LocalConstant constant = reader.GetLocalConstant(constantHandle);
                string value = Constant(reader.GetBlobReader(constant.Signature));
                line($"scope-constant scope={id} id={Row(constantHandle)} "
                    + $"name={Text(reader, constant.Name)} {value}");
            }
        }
    }

    /// <summary>Decode a local constant signature into the dump text.</summary>
    private static string Constant(BlobReader blob)
    {
        var text = new StringBuilder();
        byte code;
        while (true)
        {
            code = blob.ReadByte();
            if (code != 0x1f && code != 0x20)
            {
                break;
            }
            string kind = code == 0x1f ? "required" : "optional";
            text.Append($"modifier={kind}:{Token(blob.ReadTypeHandle())} ");
        }
        switch (code)
        {
            case 0x02:
                byte boolean = blob.ReadByte();
                if (boolean > 1)
                {
                    throw new InvalidDataException("a BOOLEAN constant is not 0 or 1");
                }
                text.Append($"boolean={(boolean == 1 ? "true" : "false")}");
                break;
            case 0x03: text.Append($"char=0x{(ushort)blob.ReadChar():x4}"); break;
            case 0x04: text.Append($"i1={blob.ReadSByte()}"); break;
            case 0x05: text.Append($"u1={blob.ReadByte()}"); break;
            case 0x06: text.Append($"i2={blob.ReadInt16()}"); break;
            case 0x07: text.Append($"u2={blob.ReadUInt16()}"); break;
            case 0x08: text.Append($"i4={blob.ReadInt32()}"); break;
            case 0x09: text.Append($"u4={blob.ReadUInt32()}"); break;
            case 0x0a: text.Append($"i8={blob.ReadInt64()}"); break;
            case 0x0b: text.Append($"u8={blob.ReadUInt64()}"); break;
            case 0x0c: text.Append($"r4=0x{BitConverter.SingleToInt32Bits(blob.ReadSingle()):x8}"); break;
            case 0x0d: text.Append($"r8=0x{BitConverter.DoubleToInt64Bits(blob.ReadDouble()):x16}"); break;
            case 0x0e:
                if (blob.RemainingBytes % 2 != 0)
                {
                    if (blob.RemainingBytes != 1 || blob.ReadByte() != 0xff)
                    {
                        throw new InvalidDataException("a STRING constant has an odd size");
                    }
                    text.Append("string=null");
                }
                else
                {
                    string value = blob.ReadUTF16(blob.RemainingBytes);
                    text.Append($"string={Dump.Quote(Encoding.UTF8.GetBytes(value))}");
                }
                break;
            case 0x11:
            case 0x12:
                string kind = code == 0x11 ? "valuetype" : "class";
                text.Append($"{kind}={Token(blob.ReadTypeHandle())} ");
                switch (blob.RemainingBytes)
                {
                    case 0: text.Append("default"); break;
                    case 8: text.Append($"datetime={blob.ReadInt64()}"); break;
                    case 13:
                        int[] bits = decimal.GetBits(blob.ReadDecimal());
                        int sign = (bits[3] >> 31) & 1;
                        int scale = (bits[3] >> 16) & 0xff;
                        text.Append($"decimal={sign}:{scale}:0x{bits[2]:x8}:0x{bits[1]:x8}:0x{bits[0]:x8}");
                        break;
                    default: throw new InvalidDataException("a general value has an unknown size");
                }
                break;
            case 0x1c: text.Append("object"); break;
            default: throw new InvalidDataException($"a local constant has the type code 0x{code:x2}");
        }
        if (code >= 0x02 && code <= 0x0b && blob.RemainingBytes > 0)
        {
            text.Append($" enum={Token(blob.ReadTypeHandle())}");
        }
        if (blob.RemainingBytes > 0)
        {
            throw new InvalidDataException("a local constant signature has bytes after the value");
        }
        return text.ToString();
    }

    private static void WriteImportScopes(MetadataReader reader, Action<string> line)
    {
        foreach (ImportScopeHandle handle in reader.ImportScopes)
        {
            ImportScope scope = reader.GetImportScope(handle);
            int id = Row(handle);
            int count = 0;
            foreach (ImportDefinition _ in scope.GetImports())
            {
                count++;
            }
            line($"import-scope id={id} parent={Row(scope.Parent)} imports={count}");
            foreach (ImportDefinition import in scope.GetImports())
            {
                int kind = (int)import.Kind;
                var text = new StringBuilder($"import scope={id} kind={kind}");
                if (kind is 4 or 5 or 6 or 7 or 8 or 9)
                {
                    text.Append($" alias={Dump.Quote(reader.GetBlobBytes(import.Alias))}");
                }
                if (kind is 2 or 6 or 8)
                {
                    text.Append($" assembly={Row(import.TargetAssembly)}");
                }
                if (kind is 1 or 2 or 4 or 7 or 8)
                {
                    text.Append($" namespace={Dump.Quote(reader.GetBlobBytes(import.TargetNamespace))}");
                }
                if (kind is 3 or 9)
                {
                    text.Append($" type={Token(import.TargetType)}");
                }
                line(text.ToString());
            }
        }
    }

    private static void WriteStateMachines(MetadataReader reader, Action<string> line)
    {
        foreach (MethodDebugInformationHandle handle in reader.MethodDebugInformation)
        {
            MethodDefinitionHandle kickoff =
                reader.GetMethodDebugInformation(handle).GetStateMachineKickoffMethod();
            if (!kickoff.IsNil)
            {
                line($"state-machine move-next={Row(handle)} kickoff={Row(kickoff)}");
            }
        }
    }

    private static int Row(EntityHandle handle) => handle.IsNil ? 0 : MetadataTokens.GetRowNumber(handle);

    private static int Row(MethodDebugInformationHandle handle) => MetadataTokens.GetRowNumber(handle);

    private static string Token(EntityHandle handle) => $"0x{MetadataTokens.GetToken(handle):x8}";

    private static string Text(MetadataReader reader, StringHandle handle) =>
        Dump.Quote(Encoding.UTF8.GetBytes(reader.GetString(handle)));

    /// <summary>A GUID in text form; the nil index gives the empty GUID.</summary>
    private static string Guid(MetadataReader reader, GuidHandle handle) =>
        (handle.IsNil ? System.Guid.Empty : reader.GetGuid(handle)).ToString("D");

    private static string HexBytes(byte[] bytes) => Convert.ToHexString(bytes).ToLowerInvariant();
}
