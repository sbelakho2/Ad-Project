/**
 * SCTE-35 Parser
 *
 * Parses SCTE-35 binary messages for ad break signaling.
 * Supports splice_insert and time_signal commands.
 */

import {
  SCTE35Message,
  SpliceCommandType,
  SpliceInsert,
  TimeSignal,
  SpliceTime,
  BreakDuration,
  SpliceDescriptor,
  SegmentationDescriptor,
  SegmentationType,
  SEGMENTATION_TYPE_IDS,
  AdBreakCue,
} from './types';

/**
 * SCTE-35 command type codes
 */
const SPLICE_COMMAND_TYPES: Record<number, SpliceCommandType> = {
  0x00: 'splice_null',
  0x04: 'splice_insert',
  0x05: 'bandwidth_reservation',
  0x06: 'time_signal',
  0xff: 'private_command',
};

/**
 * SCTE-35 Parser
 */
export class SCTE35Parser {
  /**
   * Parse SCTE-35 from base64 string
   */
  parse(base64Data: string): SCTE35Message {
    const data = this.decodeBase64(base64Data);
    return this.parseBuffer(data, base64Data);
  }

  /**
   * Parse SCTE-35 from hex string
   */
  parseHex(hexData: string): SCTE35Message {
    const cleanHex = hexData.replace(/^0x/i, '').replace(/\s/g, '');
    const bytes = new Uint8Array(cleanHex.length / 2);

    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = parseInt(cleanHex.substring(i * 2, i * 2 + 2), 16);
    }

    const base64 = this.encodeBase64(bytes);
    return this.parseBuffer(bytes, base64);
  }

  /**
   * Parse buffer
   */
  private parseBuffer(data: Uint8Array, rawData: string): SCTE35Message {
    let offset = 0;

    // Table ID (should be 0xFC for SCTE-35)
    const tableId = data[offset++];
    if (tableId !== 0xfc) {
      throw new Error(`Invalid SCTE-35 table ID: 0x${tableId.toString(16)}`);
    }

    // Section syntax indicator, private indicator, reserved, section length
    const sectionLengthByte1 = data[offset++];
    const sectionLengthByte2 = data[offset++];
    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    const sectionLength = ((sectionLengthByte1 & 0x0f) << 8) | sectionLengthByte2;

    // Protocol version
    const protocolVersion = data[offset++];

    // Encrypted packet, encryption algorithm, PTS adjustment
    const encryptedByte = data[offset++];
    const encryptedPacket = (encryptedByte & 0x80) !== 0;

    // PTS adjustment (33 bits)
    const ptsAdjustment = this.readBits(data, offset, 33);
    offset += 4;

    // CW index
    const cwIndex = data[offset++];

    // Tier (12 bits) and splice command length (12 bits)
    const tierByte1 = data[offset++];
    const tierByte2 = data[offset++];
    const tier = ((tierByte1 & 0x0f) << 8) | tierByte2;

    const spliceCommandLength = ((data[offset++] << 8) | data[offset++]) & 0x0fff;

    // Splice command type
    const spliceCommandTypeCode = data[offset++];
    const spliceCommandType = SPLICE_COMMAND_TYPES[spliceCommandTypeCode] || 'private_command';

    // Parse splice command
    let spliceCommand: SpliceInsert | TimeSignal | undefined;
    const commandEndOffset = offset + spliceCommandLength - 1;

    if (spliceCommandType === 'splice_insert') {
      const result = this.parseSpliceInsert(data, offset);
      spliceCommand = result.command;
      offset = result.offset;
    } else if (spliceCommandType === 'time_signal') {
      const result = this.parseTimeSignal(data, offset);
      spliceCommand = result.command;
      offset = result.offset;
    } else {
      // Skip unknown commands
      offset = Math.min(commandEndOffset, offset + spliceCommandLength - 1);
    }

    // Descriptor loop length
    const descriptorLoopLength = (data[offset++] << 8) | data[offset++];

    // Parse descriptors
    const descriptors: SpliceDescriptor[] = [];
    const descriptorEndOffset = offset + descriptorLoopLength;

    while (offset < descriptorEndOffset && offset < data.length - 4) {
      const result = this.parseDescriptor(data, offset);
      descriptors.push(result.descriptor);
      offset = result.offset;
    }

    // CRC32
    const crc32 =
      (data[data.length - 4] << 24) |
      (data[data.length - 3] << 16) |
      (data[data.length - 2] << 8) |
      data[data.length - 1];

    return {
      tableId,
      protocolVersion,
      encryptedPacket,
      ptsAdjustment,
      cwIndex,
      tier,
      spliceCommandLength,
      spliceCommandType,
      spliceCommand,
      descriptorLoopLength,
      descriptors,
      crc32,
      rawData,
    };
  }

  /**
   * Parse splice_insert command
   */
  private parseSpliceInsert(
    data: Uint8Array,
    offset: number
  ): { command: SpliceInsert; offset: number } {
    const spliceEventId =
      (data[offset++] << 24) |
      (data[offset++] << 16) |
      (data[offset++] << 8) |
      data[offset++];

    const cancelByte = data[offset++];
    const spliceEventCancelIndicator = (cancelByte & 0x80) !== 0;

    if (spliceEventCancelIndicator) {
      return {
        command: { spliceEventId, spliceEventCancelIndicator },
        offset,
      };
    }

    const flagsByte = data[offset++];
    const outOfNetworkIndicator = (flagsByte & 0x80) !== 0;
    const programSpliceFlag = (flagsByte & 0x40) !== 0;
    const durationFlag = (flagsByte & 0x20) !== 0;
    const spliceImmediateFlag = (flagsByte & 0x10) !== 0;

    let spliceTime: SpliceTime | undefined;

    if (programSpliceFlag && !spliceImmediateFlag) {
      const result = this.parseSpliceTime(data, offset);
      spliceTime = result.spliceTime;
      offset = result.offset;
    }

    let breakDuration: BreakDuration | undefined;

    if (durationFlag) {
      const result = this.parseBreakDuration(data, offset);
      breakDuration = result.breakDuration;
      offset = result.offset;
    }

    const uniqueProgramId = (data[offset++] << 8) | data[offset++];
    const availNum = data[offset++];
    const availsExpected = data[offset++];

    return {
      command: {
        spliceEventId,
        spliceEventCancelIndicator,
        outOfNetworkIndicator,
        programSpliceFlag,
        durationFlag,
        spliceImmediateFlag,
        spliceTime,
        breakDuration,
        uniqueProgramId,
        availNum,
        availsExpected,
      },
      offset,
    };
  }

  /**
   * Parse time_signal command
   */
  private parseTimeSignal(
    data: Uint8Array,
    offset: number
  ): { command: TimeSignal; offset: number } {
    const result = this.parseSpliceTime(data, offset);
    return {
      command: { spliceTime: result.spliceTime },
      offset: result.offset,
    };
  }

  /**
   * Parse splice time
   */
  private parseSpliceTime(
    data: Uint8Array,
    offset: number
  ): { spliceTime: SpliceTime; offset: number } {
    const timeSpecifiedFlag = (data[offset] & 0x80) !== 0;

    if (!timeSpecifiedFlag) {
      offset += 1;
      return {
        spliceTime: { timeSpecifiedFlag: false },
        offset,
      };
    }

    // PTS time is 33 bits
    const ptsTime =
      ((data[offset] & 0x01) * 0x100000000) +
      (data[offset + 1] << 24) +
      (data[offset + 2] << 16) +
      (data[offset + 3] << 8) +
      data[offset + 4];

    offset += 5;

    return {
      spliceTime: { timeSpecifiedFlag: true, ptsTime },
      offset,
    };
  }

  /**
   * Parse break duration
   */
  private parseBreakDuration(
    data: Uint8Array,
    offset: number
  ): { breakDuration: BreakDuration; offset: number } {
    const autoReturn = (data[offset] & 0x80) !== 0;

    // Duration is 33 bits
    const duration =
      ((data[offset] & 0x01) * 0x100000000) +
      (data[offset + 1] << 24) +
      (data[offset + 2] << 16) +
      (data[offset + 3] << 8) +
      data[offset + 4];

    offset += 5;

    return {
      breakDuration: { autoReturn, duration },
      offset,
    };
  }

  /**
   * Parse descriptor
   */
  private parseDescriptor(
    data: Uint8Array,
    offset: number
  ): { descriptor: SpliceDescriptor; offset: number } {
    const tag = data[offset++];
    const length = data[offset++];

    // Identifier (4 bytes, should be "CUEI" = 0x43554549)
    const identifier = String.fromCharCode(
      data[offset],
      data[offset + 1],
      data[offset + 2],
      data[offset + 3]
    );
    offset += 4;

    const descriptorData = data.slice(offset, offset + length - 4);
    offset += length - 4;

    const descriptor: SpliceDescriptor = {
      tag,
      length,
      identifier,
      data: descriptorData,
    };

    // Parse segmentation descriptor (tag 0x02)
    if (tag === 0x02 && identifier === 'CUEI') {
      descriptor.segmentation = this.parseSegmentationDescriptor(descriptorData);
    }

    return { descriptor, offset };
  }

  /**
   * Parse segmentation descriptor
   */
  private parseSegmentationDescriptor(data: Uint8Array): SegmentationDescriptor {
    let offset = 0;

    const segmentationEventId =
      (data[offset++] << 24) |
      (data[offset++] << 16) |
      (data[offset++] << 8) |
      data[offset++];

    const cancelByte = data[offset++];
    const segmentationEventCancelIndicator = (cancelByte & 0x80) !== 0;

    if (segmentationEventCancelIndicator) {
      return {
        segmentationEventId,
        segmentationEventCancelIndicator,
        segmentationTypeId: 0,
        segmentationType: 'not_indicated',
      };
    }

    const flagsByte = data[offset++];
    const programSegmentationFlag = (flagsByte & 0x80) !== 0;
    const segmentationDurationFlag = (flagsByte & 0x40) !== 0;
    const deliveryNotRestrictedFlag = (flagsByte & 0x20) !== 0;

    let webDeliveryAllowedFlag: boolean | undefined;
    let noRegionalBlackoutFlag: boolean | undefined;
    let archiveAllowedFlag: boolean | undefined;
    let deviceRestrictions: number | undefined;

    if (!deliveryNotRestrictedFlag) {
      webDeliveryAllowedFlag = (flagsByte & 0x10) !== 0;
      noRegionalBlackoutFlag = (flagsByte & 0x08) !== 0;
      archiveAllowedFlag = (flagsByte & 0x04) !== 0;
      deviceRestrictions = flagsByte & 0x03;
    }

    let segmentationDuration: number | undefined;

    if (segmentationDurationFlag) {
      // 40 bits
      segmentationDuration =
        data[offset++] * 0x100000000 +
        (data[offset++] << 24) +
        (data[offset++] << 16) +
        (data[offset++] << 8) +
        data[offset++];
    }

    const segmentationUpidType = data[offset++];
    const segmentationUpidLength = data[offset++];
    const segmentationUpid = data.slice(offset, offset + segmentationUpidLength);
    offset += segmentationUpidLength;

    const segmentationTypeId = data[offset++];
    const segmentationType = SEGMENTATION_TYPE_IDS[segmentationTypeId] || 'not_indicated';

    const segmentNum = data[offset++];
    const segmentsExpected = data[offset++];

    let subSegmentNum: number | undefined;
    let subSegmentsExpected: number | undefined;

    if (offset < data.length - 1) {
      subSegmentNum = data[offset++];
      subSegmentsExpected = data[offset++];
    }

    return {
      segmentationEventId,
      segmentationEventCancelIndicator,
      programSegmentationFlag,
      segmentationDurationFlag,
      deliveryNotRestrictedFlag,
      webDeliveryAllowedFlag,
      noRegionalBlackoutFlag,
      archiveAllowedFlag,
      deviceRestrictions,
      segmentationDuration,
      segmentationUpidType,
      segmentationUpid,
      segmentationTypeId,
      segmentationType,
      segmentNum,
      segmentsExpected,
      subSegmentNum,
      subSegmentsExpected,
    };
  }

  /**
   * Convert SCTE-35 to normalized ad break cue
   */
  toAdBreakCue(message: SCTE35Message, mediaTimeSeconds: number): AdBreakCue | null {
    // Determine cue type from segmentation descriptors
    let cueType: 'start' | 'end' = 'start';
    let breakDuration: number | undefined;
    let segmentationType: SegmentationType | undefined;
    let autoReturn = false;
    let availNum: number | undefined;
    let availsExpected: number | undefined;
    let breakId: string | undefined;

    // Check segmentation descriptors
    for (const desc of message.descriptors) {
      if (desc.segmentation) {
        const seg = desc.segmentation;
        segmentationType = seg.segmentationType;

        // Determine if this is a start or end cue
        if (segmentationType.includes('_end') || segmentationType.includes('_in')) {
          cueType = 'end';
        }

        // Extract duration if available (convert from 90kHz to seconds)
        if (seg.segmentationDuration) {
          breakDuration = seg.segmentationDuration / 90000;
        }

        breakId = seg.segmentationEventId.toString();
      }
    }

    // Check splice_insert command
    if (message.spliceCommandType === 'splice_insert') {
      const cmd = message.spliceCommand as SpliceInsert;

      if (cmd.outOfNetworkIndicator === false) {
        cueType = 'end';
      }

      if (cmd.breakDuration) {
        breakDuration = cmd.breakDuration.duration / 90000;
        autoReturn = cmd.breakDuration.autoReturn;
      }

      availNum = cmd.availNum;
      availsExpected = cmd.availsExpected;
      breakId = cmd.spliceEventId?.toString();
    }

    return {
      cueId: `scte35_${breakId || Date.now()}`,
      type: cueType,
      mediaTime: mediaTimeSeconds,
      duration: breakDuration,
      breakId,
      autoReturn,
      availNum,
      availsExpected,
      segmentationType,
      rawScte35: message.rawData,
      source: 'scte35',
      original: message,
    };
  }

  /**
   * Helper to read bits from buffer
   */
  private readBits(data: Uint8Array, offset: number, bits: number): number {
    // Simplified - handles common cases
    if (bits === 33) {
      return (
        ((data[offset] & 0x01) * 0x100000000) +
        (data[offset + 1] << 24) +
        (data[offset + 2] << 16) +
        (data[offset + 3] << 8) +
        data[offset + 4]
      );
    }
    return 0;
  }

  /**
   * Decode base64 to Uint8Array
   */
  private decodeBase64(base64: string): Uint8Array {
    const binaryString = atob(base64);
    const bytes = new Uint8Array(binaryString.length);
    for (let i = 0; i < binaryString.length; i++) {
      bytes[i] = binaryString.charCodeAt(i);
    }
    return bytes;
  }

  /**
   * Encode Uint8Array to base64
   */
  private encodeBase64(bytes: Uint8Array): string {
    let binaryString = '';
    for (let i = 0; i < bytes.length; i++) {
      binaryString += String.fromCharCode(bytes[i]);
    }
    return btoa(binaryString);
  }
}

// Export singleton instance
export const scte35Parser = new SCTE35Parser();
