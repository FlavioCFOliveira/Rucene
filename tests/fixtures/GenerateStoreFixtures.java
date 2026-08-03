// Fixture generator for Rucene store-layer portability tests.
//
// This program uses Apache Lucene Core 10.5.0 classes to produce a known
// binary payload and its CRC-32 checksum. The values it prints are embedded
// as reference data in tests/store_portability.rs. Re-run this generator
// whenever the payload shape changes and copy the new constants into the
// Rust tests.
//
// Build requirements:
//   - JRE 21 (or compatible)
//   - Eclipse ECJ compiler jar (ecj-*.jar)
//   - lucene-core-10.5.0.jar
//
// Example usage:
//   java -jar ecj-4.32.jar -source 21 -target 21 \
//       -cp lucene-core-10.5.0.jar -d . GenerateStoreFixtures.java
//   java -cp ".:lucene-core-10.5.0.jar" GenerateStoreFixtures

import org.apache.lucene.store.ByteBuffersDirectory;
import org.apache.lucene.store.Directory;
import org.apache.lucene.store.IndexOutput;
import org.apache.lucene.store.IndexInput;
import org.apache.lucene.store.IOContext;
import java.util.zip.CRC32;
import java.util.Base64;

public class GenerateStoreFixtures {
    public static void main(String[] args) throws Exception {
        try (Directory dir = new ByteBuffersDirectory()) {
            try (IndexOutput out = dir.createOutput("test.bin", IOContext.DEFAULT)) {
                // Mirror the sequence written by Rucene portability tests.
                out.writeByte((byte) 0x01);
                out.writeShort((short) 0x1234);
                out.writeInt(0xDEADBEEF);
                out.writeLong(0x1234567890ABCDEFL);
                out.writeVInt(16384);
                out.writeVLong(1_000_000_000_000L);
                out.writeString("Rucene portability test");
                out.writeBytes(new byte[] { 0x0A, 0x0B, 0x0C, 0x0D }, 4);
            }

            byte[] fileBytes;
            long javaChecksum;
            try (IndexInput in = dir.openInput("test.bin", IOContext.DEFAULT)) {
                fileBytes = new byte[(int) in.length()];
                in.readBytes(fileBytes, 0, fileBytes.length);
                CRC32 crc = new CRC32();
                crc.update(fileBytes, 0, fileBytes.length);
                javaChecksum = crc.getValue();
            }

            System.out.println("payload_base64=" + Base64.getEncoder().encodeToString(fileBytes));
            System.out.println("payload_length=" + fileBytes.length);
            System.out.println("crc32=" + javaChecksum);

            try (IndexOutput out = dir.createOutput("hello.txt", IOContext.DEFAULT)) {
                out.writeString("Hello Lucene");
            }
            try (IndexInput in = dir.openInput("hello.txt", IOContext.DEFAULT)) {
                byte[] helloBytes = new byte[(int) in.length()];
                in.readBytes(helloBytes, 0, helloBytes.length);
                CRC32 crc = new CRC32();
                crc.update(helloBytes, 0, helloBytes.length);
                System.out.println("hello_base64=" + Base64.getEncoder().encodeToString(helloBytes));
                System.out.println("hello_crc32=" + crc.getValue());
            }
        }
    }
}
