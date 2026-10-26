' A function whose sequence points change documents (#23). Two #ExternalSource blocks map parts
' of the body to the documents Mapped.vb and MappedNext.vb. The VB compiler hides the lines of a
' file outside its #ExternalSource blocks, so the function is in its own file and has a block for
' each document. Compute returns its argument, so the checksum does not change.
Module MappedRegion

    Function Compute(value As Integer) As Integer
        Dim result As Integer = value + 1
#ExternalSource("Mapped.vb", 100)
        result *= 2
        result \= 2
#End ExternalSource
#ExternalSource("MappedNext.vb", 200)
        result -= 1
#End ExternalSource
        Return result
    End Function

End Module
